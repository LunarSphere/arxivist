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

## Images

The repository root `Dockerfile` builds any Rust service by binary name. Build images from the
repository root:

```bash
docker build --build-arg BIN=arxivist-crawler -t arxivist-crawler:local .
docker build --build-arg BIN=arxivist-indexer -t arxivist-indexer:local .
docker build --build-arg BIN=arxivist-search-api -t arxivist-search-api:local .
```

After `cdk deploy`, use the ECR repository outputs to tag and push the images.

## Application Adapter Gap

The current Rust binaries still use local filesystem inputs/outputs. This stack prepares the AWS resources and runtime environment variables, but the services still need AWS adapters before the full crawl/index/search pipeline can run against S3, DynamoDB, and SQS.

The crawler now uses Spider and respects robots.txt locally. Before running it as a long-lived cloud crawler, add SQS frontier consumption, S3 content writes, DynamoDB crawl metadata writes, and a controlled ECS trigger such as a scheduled task or one-shot task command with explicit seeds.
