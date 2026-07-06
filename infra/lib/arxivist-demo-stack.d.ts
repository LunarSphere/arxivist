import { Stack, StackProps } from "aws-cdk-lib";
import { Construct } from "constructs";
interface ArxivistDemoStackProps extends StackProps {
    projectName: string;
}
export declare class ArxivistDemoStack extends Stack {
    constructor(scope: Construct, id: string, props: ArxivistDemoStackProps);
    private addGlobalCostTags;
    private addServiceTags;
    private repository;
    private workerTask;
}
export {};
