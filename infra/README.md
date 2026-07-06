# Arxivist Infra

CDK app for the AWS backend. The frontend is deployed separately from `../frontend` to Vercel and
uses its own `/api` proxy to call this backend.

## What This Deploys

- S3 for crawl snapshots and versioned index artifacts.
- DynamoDB tables for page metadata and crawl URL de-duplication.
- SQS crawl frontier queue plus a dead-letter queue.
- ECR repositories for the crawler, indexer, and search API images.
- ECS Fargate task definitions for crawler/indexer and a public ALB-backed search API service.
- Lambda plus HTTP API Gateway for the agentic search API.
- CloudWatch logs and a DLQ alarm.
- Optional AWS Budget email alert.

The stack avoids NAT gateways. By default, the search API desired count is `0`, so a fresh deploy
does not keep paid compute running unless you pass `-c searchDesiredCount=1`.

## Deployment Order

Use this order for a fresh AWS deployment:

1. Configure the shell environment and confirm AWS identity.
2. Store the OpenAI API key in Secrets Manager.
3. Install/build/synth the CDK app and bootstrap the account if needed.
4. Deploy CDK. This creates AWS resources, ECR repositories, and the agent Lambda image asset.
5. Build and push the Rust ECS images for crawler, indexer, and search API.
6. Crawl pages, run the indexer, and confirm `indexes/active/manifest.json` exists in S3.
7. Scale the search API to one task and verify `/health` and `/search`.
8. Verify the agent API through API Gateway.
9. Point Vercel at the AWS outputs and redeploy the frontend.

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

Confirm the account before deploying:

```bash
aws sts get-caller-identity
```

Store the OpenAI API key in AWS Secrets Manager before using the agent API. The CDK stack imports
this secret by name and grants the Lambda read access; it does not store the key in source or CDK
context.

```bash
aws secretsmanager create-secret \
  --name "$PROJECT_NAME/openai-api-key" \
  --secret-string "$OPENAI_API_KEY"
```

If the secret already exists, rotate/update it instead:

```bash
aws secretsmanager put-secret-value \
  --secret-id "$PROJECT_NAME/openai-api-key" \
  --secret-string "$OPENAI_API_KEY"
```

The secret name defaults to `$PROJECT_NAME/openai-api-key`. Override it during deploy only if you
also created a matching secret:

```bash
-c openAiApiKeySecretName=your/custom/secret-name
```

Useful outputs after deploy:

```bash
cd infra
export SEARCH_API_URL=$(node -e 'const o = require("./cdk-outputs.json"); console.log(o.ArxivistDemoStack.SearchApiUrl)')
export AGENT_API_URL=$(node -e 'const o = require("./cdk-outputs.json"); console.log(o.ArxivistDemoStack.AgentApiUrl)')
export CRAWL_QUEUE_URL=$(node -e 'const o = require("./cdk-outputs.json"); console.log(o.ArxivistDemoStack.CrawlQueueUrl)')
```

## One-Time AWS Setup

Install/build the CDK app. Docker must be running because CDK builds the agent Lambda container
asset during deploy.

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
`searchDesiredCount=1` when you want the public search API to stay online after deploy. The agent
Lambda is deployed by CDK and does not need a separate manual image push.

```bash
npm run deploy -- \
  -c projectName=$PROJECT_NAME \
  -c demoCorsOrigin=$DEMO_CORS_ORIGIN \
  -c budgetEmail=$BUDGET_EMAIL \
  -c searchDesiredCount=1 \
  --outputs-file cdk-outputs.json
```

## Cost Allocation Tags

The stack applies these tags to taggable resources:

```text
Project=arxivist
Environment=demo
ManagedBy=cdk
Owner=kevius
CostCenter=arxivist-demo
```

It also adds service-level tags so AWS costs can be grouped by subsystem:

```text
Service=storage
Service=metadata
Service=queue
Service=crawler
Service=indexer
Service=search-api
Service=agent-api
Service=network
Service=compute
Service=observability
```

Each tagged resource also gets a narrower `Component` tag such as `crawl-snapshots`,
`pages-table`, `crawl-frontier`, `worker-task`, or `public-load-balancer`.

After deploying tag changes, activate these user-defined cost allocation tags in AWS Billing:

```text
Project
Environment
ManagedBy
Owner
CostCenter
Service
Component
```

AWS Cost Explorer can take up to 24 hours to start showing newly activated tags.

## Build And Push Images

Run from the repository root after CDK has created the ECR repositories. These images are only for
the Rust ECS services; the Python agent Lambda image is built and published by CDK.

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
  --overrides '{"containerOverrides":[{"name":"Worker","command":["--storage","aws","--crawl-id","broad-web-25k-20260704","--seed","https://en.wikipedia.org/wiki/Main_Page?arxivist_run=broad20260704","--seed","https://www.gutenberg.org/?arxivist_run=broad20260704","--seed","https://openlibrary.org/?arxivist_run=broad20260704","--seed","https://www.nasa.gov/?arxivist_run=broad20260704","--seed","https://www.noaa.gov/?arxivist_run=broad20260704","--seed","https://www.loc.gov/?arxivist_run=broad20260704","--seed","https://www.si.edu/?arxivist_run=broad20260704","--seed","https://docs.python.org/3/?arxivist_run=broad20260704","--seed","https://developer.mozilla.org/en-US/?arxivist_run=broad20260704","--seed","https://arxiv.org/?arxivist_run=broad20260704","--max-pages","25000","--max-depth","8","--delay-ms","250","--concurrency","4"]}]}' \
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
aws s3 ls "s3://$PROJECT_NAME-data-$CDK_DEFAULT_ACCOUNT-$AWS_REGION/indexes/active/manifest.json"
```

For new crawls, page text and link payloads are stored in S3 under `crawl/extracted/`; DynamoDB
page items should contain compact metadata and an `extracted_payload_path` pointer instead of full
extracted text.

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

## Start And Verify The Agent API

The agent API is deployed as Lambda behind HTTP API Gateway. It calls the Rust search API through
`ARXIVIST_SEARCH_API_BASE_URL`, so the search ECS service should be scaled to `1` before testing
agentic search.

```bash
curl "$AGENT_API_URL/agent/health"
curl -s -X POST "$AGENT_API_URL/agent/search" \
  -H 'content-type: application/json' \
  -d '{"query":"transformer retrieval","top_k":5}'
```

If the agent returns an OpenAI authentication error, confirm that `$PROJECT_NAME/openai-api-key`
exists in Secrets Manager and contains the current API key as either a raw secret string or JSON
with an `OPENAI_API_KEY` field.

## Connect Vercel Frontend

The frontend should call Vercel's local API proxy, not the ALB directly. Set these Vercel
environment variables for the frontend project. The Vercel project root directory must be
`frontend` so Vercel deploys the `frontend/api/` proxy functions.

```text
ARXIVIST_API_BASE_URL=/api
ARXIVIST_AGENT_API_BASE_URL=/api
ARXIVIST_UPSTREAM_API_BASE_URL=<your SEARCH_API_URL>
ARXIVIST_UPSTREAM_AGENT_API_BASE_URL=<your AGENT_API_URL>
```

Example:

```text
ARXIVIST_API_BASE_URL=/api
ARXIVIST_AGENT_API_BASE_URL=/api
ARXIVIST_UPSTREAM_API_BASE_URL=http://arxivist-search-api-911169521.us-east-1.elb.amazonaws.com
ARXIVIST_UPSTREAM_AGENT_API_BASE_URL=https://abc123.execute-api.us-east-1.amazonaws.com
```

Why both variables exist:

- `ARXIVIST_API_BASE_URL` is baked into `frontend/dist/config.js` at build time. `/api` makes the
  browser call the same Vercel origin.
- `ARXIVIST_AGENT_API_BASE_URL` is also baked into `frontend/dist/config.js`. `/api` keeps agent
  search on the same Vercel origin.
- `ARXIVIST_UPSTREAM_API_BASE_URL` is read by the files in `frontend/api/` at request time. Those
  Vercel functions forward `GET /api/health` and `POST /api/search` to AWS.
- `ARXIVIST_UPSTREAM_AGENT_API_BASE_URL` is read at request time and forwards
  `GET /api/agent/health` and `POST /api/agent/search` to AWS.

After setting or changing these variables, redeploy the Vercel frontend. Then verify:

```bash
curl "https://your-vercel-app.vercel.app/api/health"
curl -s -X POST "https://your-vercel-app.vercel.app/api/search" \
  -H 'content-type: application/json' \
  -d '{"query":"transformer","top_k":5,"mode":"traditional"}'
curl -s -X POST "https://your-vercel-app.vercel.app/api/agent/search" \
  -H 'content-type: application/json' \
  -d '{"query":"transformer retrieval","top_k":5}'
```

If Vercel returns `404`, the frontend project root is probably not set to `frontend`, or the latest
commit with the `frontend/api/` proxy files has not been redeployed. If Vercel returns `500` with
`ARXIVIST_UPSTREAM_API_BASE_URL is not configured` or
`ARXIVIST_UPSTREAM_AGENT_API_BASE_URL is not configured`, the matching upstream environment
variable is missing from the Vercel deployment. If traditional search returns `502`, check
`$SEARCH_API_URL/health` directly and make sure the ECS service desired count is `1`. If agent
search returns `502`, check `$AGENT_API_URL/agent/health` and the Lambda logs.

## Troubleshooting

- `503 Service Temporarily Unavailable` from `$SEARCH_API_URL`: the ALB has no healthy targets.
  Check `aws ecs describe-services --cluster "$PROJECT_NAME-cluster" --services "$PROJECT_NAME-search-api"`.
- `desired=0` on the search service: scale it to `1`, or redeploy CDK with `-c searchDesiredCount=1`.
- Crawler exits immediately with `shared aws crawl budget exhausted`: use a fresh `--crawl-id`.
- Crawler starts but SQS is empty: retained URL de-dupe skipped the seeds; use new seed URLs or
  harmless query strings for a demo run.
- ECS task exits `137`: the container ran out of memory. Rebuild/push the latest image and redeploy
  the task definition.
- Agent returns an OpenAI auth error: update `$PROJECT_NAME/openai-api-key` in Secrets Manager and
  invoke the Lambda again. A redeploy is not required for secret value changes.
- Agent returns search tool errors: verify `$SEARCH_API_URL/health`, then make sure the search ECS
  service is scaled to `1`.

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
