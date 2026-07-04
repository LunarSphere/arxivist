# Arxivist Infra

CDK app for the AWS backend. The frontend is deployed separately from `../frontend` to Vercel and
uses its own `/api` proxy to call this backend.

## What This Deploys

- S3 for crawl snapshots and versioned index artifacts.
- DynamoDB tables for page metadata and crawl URL de-duplication.
- SQS crawl frontier queue plus a dead-letter queue.
- ECR repositories for the crawler, indexer, and search API images.
- ECS Fargate task definitions for crawler/indexer and a public ALB-backed search API service.
- CloudWatch logs and a DLQ alarm.
- Optional AWS Budget email alert.

The stack avoids NAT gateways. By default, the search API desired count is `0`, so a fresh deploy
does not keep paid compute running unless you pass `-c searchDesiredCount=1`.

## Environment

Run these from a shell where you deploy and operate AWS:

```bash
export AWS_PROFILE=default
export AWS_REGION=us-east-1
export CDK_DEFAULT_REGION=$AWS_REGION
export CDK_DEFAULT_ACCOUNT=$(aws sts get-caller-identity --query Account --output text)
export PROJECT_NAME=arxivist
export DEMO_CORS_ORIGIN=https://your-vercel-app.vercel.app
export BUDGET_EMAIL=you@example.com
```

Useful outputs after deploy:

```bash
cd infra
export SEARCH_API_URL=$(node -e 'const o = require("./cdk-outputs.json"); console.log(o.ArxivistDemoStack.SearchApiUrl)')
export CRAWL_QUEUE_URL=$(node -e 'const o = require("./cdk-outputs.json"); console.log(o.ArxivistDemoStack.CrawlQueueUrl)')
```

## One-Time AWS Setup

Install/build the CDK app:

```bash
cd infra
npm install
npm run build
npm run synth
```

Bootstrap CDK once per account/Region:

```bash
npx cdk bootstrap "aws://$CDK_DEFAULT_ACCOUNT/$AWS_REGION"
```

If deploy fails with `No bucket named 'cdk-hnb659fds-assets-<account>-<region>'` even though
bootstrap reports no changes, the CDK bootstrap stack still exists but its S3 asset bucket was
deleted. Recreate the missing bootstrap bucket:

```bash
aws s3api create-bucket \
  --bucket "cdk-hnb659fds-assets-$CDK_DEFAULT_ACCOUNT-$AWS_REGION" \
  --region "$AWS_REGION"
```

Deploy the infrastructure. Use `searchDesiredCount=0` while setting up images or crawling, and
`searchDesiredCount=1` when you want the public API to stay online after deploy.

```bash
npm run deploy -- \
  -c projectName=$PROJECT_NAME \
  -c demoCorsOrigin=$DEMO_CORS_ORIGIN \
  -c budgetEmail=$BUDGET_EMAIL \
  -c searchDesiredCount=1 \
  --outputs-file cdk-outputs.json
```

## Build And Push Images

Run from the repository root:

```bash
aws ecr get-login-password --region "$AWS_REGION" \
  | docker login --username AWS --password-stdin \
    "$CDK_DEFAULT_ACCOUNT.dkr.ecr.$AWS_REGION.amazonaws.com"

docker build --build-arg BIN=arxivist-crawler \
  -t "$CDK_DEFAULT_ACCOUNT.dkr.ecr.$AWS_REGION.amazonaws.com/$PROJECT_NAME-crawler:latest" .
docker build --build-arg BIN=arxivist-indexer \
  -t "$CDK_DEFAULT_ACCOUNT.dkr.ecr.$AWS_REGION.amazonaws.com/$PROJECT_NAME-indexer:latest" .
docker build --build-arg BIN=arxivist-search-api \
  -t "$CDK_DEFAULT_ACCOUNT.dkr.ecr.$AWS_REGION.amazonaws.com/$PROJECT_NAME-search-api:latest" .

docker push "$CDK_DEFAULT_ACCOUNT.dkr.ecr.$AWS_REGION.amazonaws.com/$PROJECT_NAME-crawler:latest"
docker push "$CDK_DEFAULT_ACCOUNT.dkr.ecr.$AWS_REGION.amazonaws.com/$PROJECT_NAME-indexer:latest"
docker push "$CDK_DEFAULT_ACCOUNT.dkr.ecr.$AWS_REGION.amazonaws.com/$PROJECT_NAME-search-api:latest"
```

If Docker reports an image tag like `.dkr.ecr..amazonaws.com/-indexer:latest`, your shell is missing
`CDK_DEFAULT_ACCOUNT`, `AWS_REGION`, or `PROJECT_NAME`.

## Crawl And Index

Get public subnet IDs:

```bash
export VPC_ID=$(aws ec2 describe-vpcs \
  --filters "Name=tag:Name,Values=$PROJECT_NAME-vpc" \
  --query 'Vpcs[0].VpcId' \
  --output text)
export SUBNET_IDS=$(aws ec2 describe-subnets \
  --filters "Name=vpc-id,Values=$VPC_ID" "Name=tag:aws-cdk:subnet-type,Values=Public" \
  --query 'Subnets[].SubnetId' \
  --output text | tr '\t' ',')
```

Run crawler workers. Use a fresh `--crawl-id` for each run. If the retained crawl URL table has
already seen a seed URL, add a harmless query string so it is enqueued again.

```bash
export CRAWLER_TASKS=$(aws ecs run-task \
  --cluster "$PROJECT_NAME-cluster" \
  --launch-type FARGATE \
  --task-definition "$PROJECT_NAME-crawler" \
  --count 4 \
  --network-configuration "awsvpcConfiguration={subnets=[$SUBNET_IDS],assignPublicIp=ENABLED}" \
  --overrides '{"containerOverrides":[{"name":"Worker","command":["--storage","aws","--crawl-id","demo-5k-fresh","--seed","https://en.wikipedia.org/wiki/Main_Page?arxivist_run=demo5k","--seed","https://openlibrary.org/?arxivist_run=demo5k","--seed","https://www.imdb.com/title/tt0266308/?arxivist_run=demo5k","--seed","https://arxiv.org/abs/1706.03762?arxivist_run=demo5k","--max-pages","5000","--max-depth","8","--delay-ms","250"]}]}' \
  --query 'tasks[].taskArn' \
  --output text)
```

Watch crawler logs:

```bash
aws logs tail "/arxivist/$PROJECT_NAME" --follow
```

Stop crawler workers whenever you have enough data:

```bash
for task in $CRAWLER_TASKS; do
  aws ecs stop-task \
    --cluster "$PROJECT_NAME-cluster" \
    --task "$task" \
    --reason "manual stop, enough crawl data"
done
```

Run the indexer after crawling:

```bash
export INDEXER_TASK=$(aws ecs run-task \
  --cluster "$PROJECT_NAME-cluster" \
  --launch-type FARGATE \
  --task-definition "$PROJECT_NAME-indexer" \
  --network-configuration "awsvpcConfiguration={subnets=[$SUBNET_IDS],assignPublicIp=ENABLED}" \
  --query 'tasks[0].taskArn' \
  --output text)

aws ecs wait tasks-stopped \
  --cluster "$PROJECT_NAME-cluster" \
  --tasks "$INDEXER_TASK"
```

Confirm the active index exists:

```bash
aws s3 ls "s3://$PROJECT_NAME-data-$CDK_DEFAULT_ACCOUNT-$AWS_REGION/indexes/active/index.json"
```

## Start And Verify The Search API

If the service is scaled to zero, the ALB returns `503 Service Temporarily Unavailable`. Scale it up:

```bash
aws ecs update-service \
  --cluster "$PROJECT_NAME-cluster" \
  --service "$PROJECT_NAME-search-api" \
  --desired-count 1 \
  --force-new-deployment

aws ecs wait services-stable \
  --cluster "$PROJECT_NAME-cluster" \
  --services "$PROJECT_NAME-search-api"
```

Test the backend directly:

```bash
curl "$SEARCH_API_URL/health"
curl -s -X POST "$SEARCH_API_URL/search" \
  -H 'content-type: application/json' \
  -d '{"query":"transformer","top_k":5,"mode":"traditional"}'
```

Healthy output should include `"status":"ok"` and a document count.

## Connect Vercel Frontend

The frontend should call Vercel's local API proxy, not the ALB directly. Set these Vercel
environment variables for the frontend project. The Vercel project root directory must be
`frontend` so Vercel deploys `frontend/api/health.js` and `frontend/api/search.js` as serverless
functions.

```text
ARXIVIST_API_BASE_URL=/api
ARXIVIST_UPSTREAM_API_BASE_URL=<your SEARCH_API_URL>
```

Example:

```text
ARXIVIST_API_BASE_URL=/api
ARXIVIST_UPSTREAM_API_BASE_URL=http://arxivist-search-api-911169521.us-east-1.elb.amazonaws.com
```

Why both variables exist:

- `ARXIVIST_API_BASE_URL` is baked into `frontend/dist/config.js` at build time. `/api` makes the
  browser call the same Vercel origin.
- `ARXIVIST_UPSTREAM_API_BASE_URL` is read by the files in `frontend/api/` at request time. Those
  Vercel functions forward `GET /api/health` and `POST /api/search` to AWS.

After setting or changing these variables, redeploy the Vercel frontend. Then verify:

```bash
curl "https://your-vercel-app.vercel.app/api/health"
curl -s -X POST "https://your-vercel-app.vercel.app/api/search" \
  -H 'content-type: application/json' \
  -d '{"query":"transformer","top_k":5,"mode":"traditional"}'
```

If Vercel returns `404`, the frontend project root is probably not set to `frontend`, or the latest
commit with `frontend/api/health.js` and `frontend/api/search.js` has not been redeployed. If Vercel
returns `500` with `ARXIVIST_UPSTREAM_API_BASE_URL is not configured`, the upstream environment
variable is missing from the Vercel deployment. If Vercel returns `502`, check `$SEARCH_API_URL/health`
directly and make sure the ECS service desired count is `1`.

## Troubleshooting

- `503 Service Temporarily Unavailable` from `$SEARCH_API_URL`: the ALB has no healthy targets.
  Check `aws ecs describe-services --cluster "$PROJECT_NAME-cluster" --services "$PROJECT_NAME-search-api"`.
- `desired=0` on the search service: scale it to `1`, or redeploy CDK with `-c searchDesiredCount=1`.
- Crawler exits immediately with `shared aws crawl budget exhausted`: use a fresh `--crawl-id`.
- Crawler starts but SQS is empty: retained URL de-dupe skipped the seeds; use new seed URLs or
  harmless query strings for a demo run.
- ECS task exits `137`: the container ran out of memory. Rebuild/push the latest image and redeploy
  the task definition.

## Cleanup

Scale the search API down when you do not need the public endpoint:

```bash
aws ecs update-service \
  --cluster "$PROJECT_NAME-cluster" \
  --service "$PROJECT_NAME-search-api" \
  --desired-count 0
```

Destroy compute resources when done:

```bash
cd infra
npm run destroy
```

The data bucket, pages table, and crawl URL table are retained on destroy so crawled pages and index
artifacts can survive compute teardown. Delete retained resources manually only when you intend to
discard the corpus.
