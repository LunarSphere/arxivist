#!/bin/sh
set -eu
: "${ARXIVIST_ARTIFACT_BUCKET:?ARXIVIST_ARTIFACT_BUCKET is required}"
: "${ARXIVIST_RELEASE_ID:?ARXIVIST_RELEASE_ID is required}"
destination="/data/releases/${ARXIVIST_RELEASE_ID}"
mkdir -p "${destination}"
aws s3 sync --only-show-errors \
  "s3://${ARXIVIST_ARTIFACT_BUCKET}/releases/${ARXIVIST_RELEASE_ID}/" \
  "${destination}/"
