#!/usr/bin/env node
import * as cdk from "aws-cdk-lib";
import { ArxivistDemoStack } from "../lib/arxivist-demo-stack";

const app = new cdk.App();
const projectName = app.node.tryGetContext("projectName") ?? "arxivist";

new ArxivistDemoStack(app, "ArxivistDemoStack", {
  projectName,
  env: {
    account: process.env.CDK_DEFAULT_ACCOUNT,
    region: process.env.CDK_DEFAULT_REGION ?? "us-east-1"
  }
});
