# AWS deployment

The CDK stack is intentionally low-cost: one Fargate task per API, no NAT
Gateway, a shared public ALB, S3 for versioned releases, and EFS for the
filesystem-backed artifacts used at runtime.

## Bootstrap

```bash
cd infra
npm install
npx cdk bootstrap aws://ACCOUNT_ID/us-east-1
```

Deploy once with a release ID that will be uploaded next:

```bash
npx cdk deploy -c releaseId=2026-07-11
```

Set the generated `arxivist/openai-api-key` secret value to the raw OpenAI API
key before using agent search. Copy the generated
`arxivist/proxy-shared-secret` value into Vercel as
`ARXIVIST_UPSTREAM_SHARED_SECRET`.

## Release a local crawl and index

Use the `ArtifactBucketName` stack output and a unique release ID. The release
must contain `index/` and `crawl/` directories.

```bash
release_id=2026-07-11
bucket=STACK_OUTPUT_ARTIFACT_BUCKET
aws s3 sync crates/data/dev/index "s3://$bucket/releases/$release_id/index/"
aws s3 sync crates/data/dev/crawl "s3://$bucket/releases/$release_id/crawl/"
```

Run the `ArtifactSyncTaskDefinitionArn` stack output in the ECS cluster with
the `ARXIVIST_RELEASE_ID=$release_id` container environment override. Use the
`ArtifactSyncSecurityGroupId` output in that task's `awsvpcConfiguration`. Wait
for the task to finish successfully, then roll services onto that EFS release:

```bash
npx cdk deploy -c releaseId=$release_id
```

## Vercel

Set the frontend project root to `frontend` and configure these production
variables:

```text
ARXIVIST_API_BASE_URL=/api
ARXIVIST_AGENT_API_BASE_URL=/api
ARXIVIST_UPSTREAM_API_BASE_URL=https://API_ORIGIN_URL
ARXIVIST_UPSTREAM_AGENT_API_BASE_URL=https://API_ORIGIN_URL
ARXIVIST_UPSTREAM_SHARED_SECRET=VALUE_FROM_SECRETS_MANAGER
```

Use the stack's `ApiOriginUrl` CloudFront output rather than the ALB URL. The
browser talks only to `/api`; Vercel adds the shared credential while
forwarding over HTTPS. Do not expose that credential through `config.js` or a
`VITE_` variable.
