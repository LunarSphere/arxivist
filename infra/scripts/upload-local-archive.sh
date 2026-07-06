#!/usr/bin/env bash
set -euo pipefail

ARCHIVE_DIR=""
PROJECT_NAME="${PROJECT_NAME:-arxivist-demo}"
AWS_REGION="${AWS_REGION:-${CDK_DEFAULT_REGION:-us-east-1}}"
AWS_ACCOUNT_ID="${AWS_ACCOUNT_ID:-${CDK_DEFAULT_ACCOUNT:-}}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --archive-dir)
      ARCHIVE_DIR="$2"
      shift 2
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

if [[ -z "$ARCHIVE_DIR" ]]; then
  echo "--archive-dir is required" >&2
  exit 2
fi

if [[ ! -f "${ARCHIVE_DIR}/pages.jsonl" ]]; then
  echo "${ARCHIVE_DIR}/pages.jsonl does not exist" >&2
  exit 2
fi

if [[ -z "$AWS_ACCOUNT_ID" ]]; then
  AWS_ACCOUNT_ID="$(aws sts get-caller-identity --query Account --output text)"
fi

DATA_BUCKET="${PROJECT_NAME}-data-${AWS_ACCOUNT_ID}-${AWS_REGION}"
PAGES_TABLE="${PROJECT_NAME}-pages"

if [[ -d "${ARCHIVE_DIR}/content" ]]; then
  aws s3 sync "${ARCHIVE_DIR}/content/" "s3://${DATA_BUCKET}/crawl/content/" --region "$AWS_REGION"
fi

if [[ -d "${ARCHIVE_DIR}/extracted" ]]; then
  aws s3 sync "${ARCHIVE_DIR}/extracted/" "s3://${DATA_BUCKET}/crawl/extracted/" --region "$AWS_REGION"
fi

python3 - "$ARCHIVE_DIR" "$PAGES_TABLE" "$AWS_REGION" <<'PY'
import hashlib
import json
import subprocess
import sys

archive_dir, table, region = sys.argv[1:4]
requests = []

def s_value(value):
    return {"S": str(value)}

def n_value(value):
    return {"N": str(value)}

def optional_string(item, name, value):
    if value:
        item[name] = s_value(value)

def optional_number(item, name, value):
    if value is not None:
        item[name] = n_value(value)

with open(f"{archive_dir}/pages.jsonl", "r", encoding="utf-8") as handle:
    for line in handle:
        if not line.strip():
            continue
        record = json.loads(line)
        requested_url = record["requested_url"]
        url_hash = hashlib.sha256(requested_url.encode("utf-8")).hexdigest()
        content_path = record.get("content_path")
        extracted_path = record.get("extracted_payload_path")

        if content_path and not content_path.startswith("crawl/"):
            content_path = f"crawl/{content_path}"
        if extracted_path and not extracted_path.startswith("crawl/"):
            extracted_path = f"crawl/{extracted_path}"

        item = {
            "url_hash": s_value(url_hash),
            "requested_url": s_value(requested_url),
            "outcome": s_value(record.get("outcome", "stored")),
            "fetched_at_ms": n_value(record.get("fetched_at_ms", 0)),
        }
        optional_string(item, "final_url", record.get("final_url"))
        optional_string(item, "title", record.get("title"))
        optional_string(item, "content_hash", record.get("content_hash"))
        optional_string(item, "content_path", content_path)
        optional_string(item, "extracted_payload_path", extracted_path)
        optional_string(item, "content_type", record.get("content_type"))
        optional_number(item, "content_length", record.get("content_length"))
        optional_number(item, "status", record.get("status"))

        requests.append({"PutRequest": {"Item": item}})

        if len(requests) == 25:
            subprocess.run(["aws", "dynamodb", "batch-write-item", "--region", region, "--request-items", json.dumps({table: requests})], check=True)
            requests.clear()

if requests:
    subprocess.run(["aws", "dynamodb", "batch-write-item", "--region", region, "--request-items", json.dumps({table: requests})], check=True)
PY

echo "uploaded local archive metadata and payloads to ${DATA_BUCKET}"
