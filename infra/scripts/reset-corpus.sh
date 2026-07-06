#!/usr/bin/env bash
set -euo pipefail

CONFIRM=0
PROJECT_NAME="${PROJECT_NAME:-arxivist-demo}"
AWS_REGION="${AWS_REGION:-${CDK_DEFAULT_REGION:-us-east-1}}"
AWS_ACCOUNT_ID="${AWS_ACCOUNT_ID:-${CDK_DEFAULT_ACCOUNT:-}}"
MANIFEST_DIR="${MANIFEST_DIR:-infra/reset-manifests}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --confirm)
      CONFIRM=1
      shift
      ;;
    --project-name)
      PROJECT_NAME="$2"
      shift 2
      ;;
    --region)
      AWS_REGION="$2"
      shift 2
      ;;
    --account-id)
      AWS_ACCOUNT_ID="$2"
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$AWS_ACCOUNT_ID" ]]; then
  AWS_ACCOUNT_ID="$(aws sts get-caller-identity --query Account --output text)"
fi

DATA_BUCKET="${PROJECT_NAME}-data-${AWS_ACCOUNT_ID}-${AWS_REGION}"
PAGES_TABLE="${PROJECT_NAME}-pages"
CRAWL_URLS_TABLE="${PROJECT_NAME}-crawl-urls"
CRAWL_QUEUE_NAME="${PROJECT_NAME}-crawl-frontier"
CRAWL_DLQ_NAME="${PROJECT_NAME}-crawl-dlq"
PREFIXES=("crawl/content/" "crawl/extracted/" "indexes/active/" "indexes/versions/")
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
MANIFEST_PATH="${MANIFEST_DIR}/reset-${RUN_ID}.json"

mkdir -p "$MANIFEST_DIR"

queue_url() {
  aws sqs get-queue-url \
    --queue-name "$1" \
    --region "$AWS_REGION" \
    --query QueueUrl \
    --output text 2>/dev/null || true
}

table_count() {
  aws dynamodb describe-table \
    --table-name "$1" \
    --region "$AWS_REGION" \
    --query 'Table.ItemCount' \
    --output text 2>/dev/null || echo 0
}

s3_count() {
  python3 - "$DATA_BUCKET" "$1" "$AWS_REGION" <<'PY' 2>/dev/null || echo 0
import json
import subprocess
import sys

bucket, prefix, region = sys.argv[1:4]
token = None
count = 0

while True:
    command = [
        "aws",
        "s3api",
        "list-objects-v2",
        "--bucket",
        bucket,
        "--prefix",
        prefix,
        "--region",
        region,
        "--output",
        "json",
    ]
    if token:
        command.extend(["--continuation-token", token])
    payload = json.loads(subprocess.check_output(command))
    count += len(payload.get("Contents", []))
    token = payload.get("NextContinuationToken")
    if not token:
        break

print(count)
PY
}

CRAWL_QUEUE_URL="$(queue_url "$CRAWL_QUEUE_NAME")"
CRAWL_DLQ_URL="$(queue_url "$CRAWL_DLQ_NAME")"
PAGES_COUNT="$(table_count "$PAGES_TABLE")"
CRAWL_URLS_COUNT="$(table_count "$CRAWL_URLS_TABLE")"

{
  printf '{\n'
  printf '  "run_id": "%s",\n' "$RUN_ID"
  printf '  "confirmed": %s,\n' "$([[ "$CONFIRM" == 1 ]] && echo true || echo false)"
  printf '  "project_name": "%s",\n' "$PROJECT_NAME"
  printf '  "region": "%s",\n' "$AWS_REGION"
  printf '  "account_id": "%s",\n' "$AWS_ACCOUNT_ID"
  printf '  "data_bucket": "%s",\n' "$DATA_BUCKET"
  printf '  "pages_table": {"name": "%s", "pre_delete_count": %s},\n' "$PAGES_TABLE" "$PAGES_COUNT"
  printf '  "crawl_urls_table": {"name": "%s", "pre_delete_count": %s},\n' "$CRAWL_URLS_TABLE" "$CRAWL_URLS_COUNT"
  printf '  "queues": {"crawl": "%s", "dlq": "%s"},\n' "$CRAWL_QUEUE_URL" "$CRAWL_DLQ_URL"
  printf '  "s3_prefixes": [\n'
  for i in "${!PREFIXES[@]}"; do
    prefix="${PREFIXES[$i]}"
    comma=","
    [[ "$i" == "$((${#PREFIXES[@]} - 1))" ]] && comma=""
    printf '    {"prefix": "%s", "pre_delete_count": %s}%s\n' "$prefix" "$(s3_count "$prefix")" "$comma"
  done
  printf '  ]\n'
  printf '}\n'
} > "$MANIFEST_PATH"

echo "wrote reset manifest: $MANIFEST_PATH"

if [[ "$CONFIRM" != 1 ]]; then
  echo "dry run only; re-run with --confirm after reviewing the manifest"
  exit 0
fi

purge_queue() {
  local url="$1"
  if [[ -n "$url" && "$url" != "None" ]]; then
    aws sqs purge-queue --queue-url "$url" --region "$AWS_REGION" || true
  fi
}

delete_s3_prefix() {
  aws s3 rm "s3://${DATA_BUCKET}/$1" --recursive --region "$AWS_REGION"
}

delete_table_items() {
  local table="$1"
  local key_name="$2"
  local start_key=""

  while true; do
    local scan_path
    scan_path="$(mktemp)"
    if [[ -n "$start_key" ]]; then
      aws dynamodb scan \
        --table-name "$table" \
        --region "$AWS_REGION" \
        --projection-expression "$key_name" \
        --exclusive-start-key "$start_key" \
        --output json > "$scan_path"
    else
      aws dynamodb scan \
        --table-name "$table" \
        --region "$AWS_REGION" \
        --projection-expression "$key_name" \
        --output json > "$scan_path"
    fi

    python3 - "$table" "$key_name" "$AWS_REGION" "$scan_path" <<'PY'
import json, subprocess, sys

table, key_name, region, scan_path = sys.argv[1:5]
with open(scan_path, "r", encoding="utf-8") as handle:
    payload = json.load(handle)
items = payload.get("Items", [])
for offset in range(0, len(items), 25):
    requests = [{"DeleteRequest": {"Key": {key_name: item[key_name]}}} for item in items[offset:offset + 25]]
    if not requests:
        continue
    subprocess.run(
        ["aws", "dynamodb", "batch-write-item", "--region", region, "--request-items", json.dumps({table: requests})],
        check=True,
    )
PY
    start_key="$(python3 - "$scan_path" <<'PY'
import json, sys
with open(sys.argv[1], "r", encoding="utf-8") as handle:
    key = json.load(handle).get("LastEvaluatedKey")
print(json.dumps(key) if key else "")
PY
)"
    rm -f "$scan_path"
    [[ -n "$start_key" ]] || break
  done
}

purge_queue "$CRAWL_QUEUE_URL"
purge_queue "$CRAWL_DLQ_URL"

for prefix in "${PREFIXES[@]}"; do
  delete_s3_prefix "$prefix"
done

delete_table_items "$PAGES_TABLE" "url_hash"
delete_table_items "$CRAWL_URLS_TABLE" "url_hash"

echo "corpus reset complete; CDK resources, ECR images, and secrets were not destroyed"
