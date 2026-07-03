# Arxivist Infra

CDK app for the demo-first AWS backend. The frontend is deployed separately from `../frontend` to Vercel.

## What This Deploys

- S3 for crawl snapshots and versioned index artifacts.
- DynamoDB tables for page metadata and crawl URL de-duplication.
- SQS crawl frontier queue plus a dead-letter queue.
- ECR repositories for the crawler, indexer, and search API images.
- ECS Fargate task definitions for crawler/indexer and a public ALB-backed search API service.
- CloudWatch logs and a DLQ alarm.
- Optional AWS Budget email alert.

The stack intentionally avoids NAT gateways and defaults the search API desired count to `0` so a fresh deploy does not keep paid compute running.

## Bootstrap

```bash
npm install
npm run build
npm run synth
```

Deploy with a Vercel origin once you know it:

```bash
npm run deploy -- \
  -c demoCorsOrigin=https://your-vercel-app.vercel.app \
  -c budgetEmail=you@example.com \
  -c searchDesiredCount=1
```

Destroy after the recording window:

```bash
npm run destroy
```

The data bucket, pages table, and crawl URL table are retained on destroy so crawled pages and
index artifacts can survive compute teardown. Delete those retained resources manually only when
you intentionally want to discard the corpus.

## Crawl Configuration

The default demo crawl is configured through CDK context in `cdk.json`:

- `crawlId`: shared crawl budget key, default `demo-50k`.
- `crawlMaxPages`: shared maximum processed pages across all crawler tasks, default `50000`.
- `crawlMaxDepth`: maximum link depth from the seed, default `8`.
- `crawlDelayMs`: per-worker fetch delay, default `250`.
- `crawlerMaxCapacity`: recommended ECS task count for parallel crawler workers, default `4`.

AWS crawler workers all pull from the same SQS frontier and reserve page slots through DynamoDB
before fetching. That means `crawlMaxPages` is a shared cap, not a per-worker cap.

## Images

The repository root `Dockerfile` builds any Rust service by binary name. Build images from the
repository root. The CDK outputs include fully qualified ECR tags after deploy.

```bash
docker build --build-arg BIN=arxivist-crawler -t arxivist-crawler:local .
docker build --build-arg BIN=arxivist-indexer -t arxivist-indexer:local .
docker build --build-arg BIN=arxivist-search-api -t arxivist-search-api:local .
```

After `cdk deploy`, use the ECR repository outputs to tag and push the images.

## Demo Pipeline

The Rust binaries keep local filesystem defaults, but the ECS task definitions run them with
`--storage aws`. In AWS mode:

- crawler consumes and produces SQS frontier messages, writes crawl records to DynamoDB, and writes stored HTML snapshots to S3.
- indexer scans DynamoDB crawl records, builds the same search index used locally, and writes both a versioned S3 artifact and `indexes/active/index.json`.
- search API loads `indexes/active/index.json` from S3 at startup and serves `/health` plus `/search` through the ALB.

One demo run is:

```bash
# 1. Deploy the stack and push all three images to the ECR repositories from the outputs.
npm run deploy -- \
  -c demoCorsOrigin=https://your-vercel-app.vercel.app \
  -c budgetEmail=you@example.com \
  -c searchDesiredCount=0

# 2. Run crawler workers with a seed override. Use --count to launch parallel SQS consumers.
aws ecs run-task \
  --cluster arxivist-cluster \
  --launch-type FARGATE \
  --task-definition arxivist-crawler \
  --count 4 \
  --network-configuration 'awsvpcConfiguration={subnets=[subnet-id],assignPublicIp=ENABLED}' \
  --overrides '{"containerOverrides":[{"name":"Worker","command":["--storage","aws","--crawl-id","demo-50k","--seed","https://books.toscrape.com/","--max-pages","50000","--max-depth","8","--delay-ms","250"]}]}'

# 3. Run the indexer task once after the crawler exits.
aws ecs run-task \
  --cluster arxivist-cluster \
  --launch-type FARGATE \
  --task-definition arxivist-indexer \
  --network-configuration 'awsvpcConfiguration={subnets=[subnet-id],assignPublicIp=ENABLED}'

# 4. Scale the search service when the active index exists.
aws ecs update-service \
  --cluster arxivist-cluster \
  --service arxivist-search-api \
  --desired-count 1
```

Use the `SearchApiUrl` output as `ARXIVIST_UPSTREAM_API_BASE_URL` in Vercel. Set
`ARXIVIST_API_BASE_URL=/api` so the browser calls the Vercel proxy over HTTPS instead of calling
the ALB directly.
