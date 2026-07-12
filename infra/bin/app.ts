#!/usr/bin/env node
import * as cdk from "aws-cdk-lib";
import { ArxivistStack } from "../lib/arxivist-stack.js";

const app = new cdk.App();
const releaseId = app.node.tryGetContext("releaseId");
if (typeof releaseId !== "string" || !releaseId.trim()) {
  throw new Error("Pass the EFS release to serve with: cdk deploy -c releaseId=<release-id>");
}
new ArxivistStack(app, "ArxivistProduction", {
  env: { account: process.env.CDK_DEFAULT_ACCOUNT, region: "us-east-1" },
  releaseId,
});
