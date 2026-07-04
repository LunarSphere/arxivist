"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.ArxivistDemoStack = void 0;
const cdk = require("aws-cdk-lib");
const aws_cdk_lib_1 = require("aws-cdk-lib");
const budgets = require("aws-cdk-lib/aws-budgets");
const cloudwatch = require("aws-cdk-lib/aws-cloudwatch");
const dynamodb = require("aws-cdk-lib/aws-dynamodb");
const ecr = require("aws-cdk-lib/aws-ecr");
const ec2 = require("aws-cdk-lib/aws-ec2");
const ecs = require("aws-cdk-lib/aws-ecs");
const elbv2 = require("aws-cdk-lib/aws-elasticloadbalancingv2");
const logs = require("aws-cdk-lib/aws-logs");
const s3 = require("aws-cdk-lib/aws-s3");
const sqs = require("aws-cdk-lib/aws-sqs");
class ArxivistDemoStack extends aws_cdk_lib_1.Stack {
    constructor(scope, id, props) {
        super(scope, id, props);
        const demoCorsOrigin = this.node.tryGetContext("demoCorsOrigin") ?? "*";
        const budgetEmail = this.node.tryGetContext("budgetEmail");
        const monthlyBudgetUsd = Number(this.node.tryGetContext("monthlyBudgetUsd") ?? 90);
        const searchDesiredCount = Number(this.node.tryGetContext("searchDesiredCount") ?? 0);
        const crawlerMaxCapacity = Number(this.node.tryGetContext("crawlerMaxCapacity") ?? 4);
        const crawlId = String(this.node.tryGetContext("crawlId") ?? "demo-50k");
        const crawlMaxPages = Number(this.node.tryGetContext("crawlMaxPages") ?? 50_000);
        const crawlMaxDepth = Number(this.node.tryGetContext("crawlMaxDepth") ?? 8);
        const crawlDelayMs = Number(this.node.tryGetContext("crawlDelayMs") ?? 250);
        const crawlEmptyReceiveLimit = Number(this.node.tryGetContext("crawlEmptyReceiveLimit") ?? 30);
        const name = (suffix) => `${props.projectName}-${suffix}`;
        // Corpus data is intentionally retained so compute can be destroyed without re-crawling.
        const dataBucket = new s3.Bucket(this, "DataBucket", {
            bucketName: `${props.projectName}-data-${this.account}-${this.region}`,
            blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
            encryption: s3.BucketEncryption.S3_MANAGED,
            enforceSSL: true,
            removalPolicy: aws_cdk_lib_1.RemovalPolicy.RETAIN,
            autoDeleteObjects: false,
            lifecycleRules: [
                {
                    id: "expire-old-index-artifacts",
                    prefix: "indexes/",
                    noncurrentVersionExpiration: aws_cdk_lib_1.Duration.days(14)
                }
            ],
            versioned: true
        });
        const pagesTable = new dynamodb.Table(this, "PagesTable", {
            tableName: name("pages"),
            partitionKey: { name: "url_hash", type: dynamodb.AttributeType.STRING },
            billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
            pointInTimeRecoverySpecification: {
                pointInTimeRecoveryEnabled: true
            },
            removalPolicy: aws_cdk_lib_1.RemovalPolicy.RETAIN
        });
        const crawlUrlsTable = new dynamodb.Table(this, "CrawlUrlsTable", {
            tableName: name("crawl-urls"),
            partitionKey: { name: "url_hash", type: dynamodb.AttributeType.STRING },
            billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
            timeToLiveAttribute: "expires_at",
            removalPolicy: aws_cdk_lib_1.RemovalPolicy.RETAIN
        });
        crawlUrlsTable.addGlobalSecondaryIndex({
            indexName: "by-status",
            partitionKey: { name: "status", type: dynamodb.AttributeType.STRING },
            sortKey: { name: "updated_at", type: dynamodb.AttributeType.STRING },
            projectionType: dynamodb.ProjectionType.ALL
        });
        const deadLetterQueue = new sqs.Queue(this, "CrawlDeadLetterQueue", {
            queueName: name("crawl-dlq"),
            retentionPeriod: aws_cdk_lib_1.Duration.days(14)
        });
        const crawlQueue = new sqs.Queue(this, "CrawlQueue", {
            queueName: name("crawl-frontier"),
            visibilityTimeout: aws_cdk_lib_1.Duration.minutes(5),
            retentionPeriod: aws_cdk_lib_1.Duration.days(4),
            deadLetterQueue: {
                queue: deadLetterQueue,
                maxReceiveCount: 3
            }
        });
        const crawlerRepository = this.repository("CrawlerRepository", name("crawler"));
        const indexerRepository = this.repository("IndexerRepository", name("indexer"));
        const searchRepository = this.repository("SearchApiRepository", name("search-api"));
        const vpc = new ec2.Vpc(this, "Vpc", {
            vpcName: name("vpc"),
            natGateways: 0,
            maxAzs: 2,
            subnetConfiguration: [
                {
                    name: "public",
                    subnetType: ec2.SubnetType.PUBLIC
                }
            ]
        });
        const cluster = new ecs.Cluster(this, "Cluster", {
            clusterName: name("cluster"),
            vpc,
            containerInsightsV2: ecs.ContainerInsights.ENABLED
        });
        const logGroup = new logs.LogGroup(this, "ServiceLogs", {
            logGroupName: `/arxivist/${props.projectName}`,
            retention: logs.RetentionDays.ONE_WEEK,
            removalPolicy: aws_cdk_lib_1.RemovalPolicy.DESTROY
        });
        const crawlerTask = this.workerTask("CrawlerTask", {
            family: name("crawler"),
            repository: crawlerRepository,
            command: [
                "--storage",
                "aws",
                "--crawl-id",
                crawlId,
                "--max-pages",
                String(crawlMaxPages),
                "--max-depth",
                String(crawlMaxDepth),
                "--delay-ms",
                String(crawlDelayMs)
            ],
            logGroup,
            environment: {
                ARXIVIST_STORAGE_MODE: "aws",
                ARXIVIST_CRAWL_ID: crawlId,
                ARXIVIST_EMPTY_RECEIVE_LIMIT: String(crawlEmptyReceiveLimit),
                ARXIVIST_DATA_BUCKET: dataBucket.bucketName,
                ARXIVIST_PAGES_TABLE: pagesTable.tableName,
                ARXIVIST_CRAWL_URLS_TABLE: crawlUrlsTable.tableName,
                ARXIVIST_CRAWL_QUEUE_URL: crawlQueue.queueUrl
            }
        });
        const indexerTask = this.workerTask("IndexerTask", {
            family: name("indexer"),
            repository: indexerRepository,
            command: ["--storage", "aws"],
            cpu: 1024,
            memoryLimitMiB: 4096,
            logGroup,
            environment: {
                ARXIVIST_STORAGE_MODE: "aws",
                ARXIVIST_DATA_BUCKET: dataBucket.bucketName,
                ARXIVIST_PAGES_TABLE: pagesTable.tableName,
                ARXIVIST_CRAWL_URLS_TABLE: crawlUrlsTable.tableName,
                ARXIVIST_ACTIVE_INDEX_KEY: "indexes/active/index.json"
            }
        });
        dataBucket.grantReadWrite(crawlerTask.taskRole);
        dataBucket.grantReadWrite(indexerTask.taskRole);
        pagesTable.grantReadWriteData(crawlerTask.taskRole);
        pagesTable.grantReadWriteData(indexerTask.taskRole);
        crawlUrlsTable.grantReadWriteData(crawlerTask.taskRole);
        crawlUrlsTable.grantReadWriteData(indexerTask.taskRole);
        crawlQueue.grantConsumeMessages(crawlerTask.taskRole);
        crawlQueue.grantSendMessages(crawlerTask.taskRole);
        const searchTask = new ecs.FargateTaskDefinition(this, "SearchTask", {
            family: name("search-api"),
            cpu: 1024,
            memoryLimitMiB: 4096
        });
        searchTask.addContainer("SearchApi", {
            image: ecs.ContainerImage.fromEcrRepository(searchRepository, "latest"),
            logging: ecs.LogDrivers.awsLogs({
                streamPrefix: "search-api",
                logGroup
            }),
            environment: {
                ARXIVIST_STORAGE_MODE: "aws",
                ARXIVIST_DATA_BUCKET: dataBucket.bucketName,
                ARXIVIST_ACTIVE_INDEX_KEY: "indexes/active/index.json",
                ARXIVIST_CORS_ORIGIN: demoCorsOrigin
            },
            command: ["--storage", "aws", "--bind", "0.0.0.0:3000"],
            portMappings: [{ containerPort: 3000 }]
        });
        dataBucket.grantRead(searchTask.taskRole);
        const searchLoadBalancer = new elbv2.ApplicationLoadBalancer(this, "SearchLoadBalancer", {
            loadBalancerName: name("search-api"),
            vpc,
            internetFacing: true
        });
        const searchListener = searchLoadBalancer.addListener("SearchHttpListener", {
            port: 80,
            open: true
        });
        // Keep the public endpoint in place while allowing demo environments to idle at zero tasks.
        const searchService = new ecs.FargateService(this, "SearchService", {
            serviceName: name("search-api"),
            cluster,
            taskDefinition: searchTask,
            desiredCount: searchDesiredCount,
            circuitBreaker: { rollback: true },
            minHealthyPercent: 100,
            assignPublicIp: true,
            vpcSubnets: { subnetType: ec2.SubnetType.PUBLIC }
        });
        const searchTargetGroup = searchListener.addTargets("SearchTargets", {
            port: 3000,
            protocol: elbv2.ApplicationProtocol.HTTP,
            targets: [searchService]
        });
        searchTargetGroup.configureHealthCheck({
            path: "/health",
            healthyHttpCodes: "200",
            interval: aws_cdk_lib_1.Duration.seconds(30)
        });
        new cloudwatch.Alarm(this, "CrawlDlqAlarm", {
            alarmName: name("crawl-dlq-visible"),
            metric: deadLetterQueue.metricApproximateNumberOfMessagesVisible(),
            threshold: 1,
            evaluationPeriods: 1
        });
        if (budgetEmail && budgetEmail.trim() !== "") {
            new budgets.CfnBudget(this, "MonthlyBudget", {
                budget: {
                    budgetName: name("monthly-demo-budget"),
                    budgetLimit: {
                        amount: monthlyBudgetUsd,
                        unit: "USD"
                    },
                    timeUnit: "MONTHLY",
                    budgetType: "COST"
                },
                notificationsWithSubscribers: [
                    {
                        notification: {
                            notificationType: "ACTUAL",
                            comparisonOperator: "GREATER_THAN",
                            threshold: 80,
                            thresholdType: "PERCENTAGE"
                        },
                        subscribers: [
                            {
                                subscriptionType: "EMAIL",
                                address: budgetEmail
                            }
                        ]
                    }
                ]
            });
        }
        new cdk.CfnOutput(this, "DataBucketName", { value: dataBucket.bucketName });
        new cdk.CfnOutput(this, "PagesTableName", { value: pagesTable.tableName });
        new cdk.CfnOutput(this, "CrawlUrlsTableName", { value: crawlUrlsTable.tableName });
        new cdk.CfnOutput(this, "CrawlQueueUrl", { value: crawlQueue.queueUrl });
        new cdk.CfnOutput(this, "SearchApiUrl", {
            value: `http://${searchLoadBalancer.loadBalancerDnsName}`
        });
        new cdk.CfnOutput(this, "CrawlerImageBuild", {
            value: `docker build --build-arg BIN=arxivist-crawler -t ${crawlerRepository.repositoryUri}:latest .`
        });
        new cdk.CfnOutput(this, "IndexerImageBuild", {
            value: `docker build --build-arg BIN=arxivist-indexer -t ${indexerRepository.repositoryUri}:latest .`
        });
        new cdk.CfnOutput(this, "SearchApiImageBuild", {
            value: `docker build --build-arg BIN=arxivist-search-api -t ${searchRepository.repositoryUri}:latest .`
        });
        new cdk.CfnOutput(this, "CrawlerMaxCapacity", { value: String(crawlerMaxCapacity) });
        new cdk.CfnOutput(this, "CrawlId", { value: crawlId });
        new cdk.CfnOutput(this, "CrawlMaxPages", { value: String(crawlMaxPages) });
    }
    repository(id, repositoryName) {
        return new ecr.Repository(this, id, {
            repositoryName,
            imageScanOnPush: true,
            removalPolicy: aws_cdk_lib_1.RemovalPolicy.DESTROY,
            emptyOnDelete: true,
            lifecycleRules: [
                {
                    maxImageCount: 5,
                    description: "Keep only recent demo images."
                }
            ]
        });
    }
    workerTask(id, props) {
        const task = new ecs.FargateTaskDefinition(this, id, {
            family: props.family,
            cpu: props.cpu ?? 512,
            memoryLimitMiB: props.memoryLimitMiB ?? 1024
        });
        task.addContainer("Worker", {
            image: ecs.ContainerImage.fromEcrRepository(props.repository, "latest"),
            command: props.command.length > 0 ? props.command : undefined,
            logging: ecs.LogDrivers.awsLogs({
                streamPrefix: props.family,
                logGroup: props.logGroup
            }),
            environment: props.environment
        });
        return task;
    }
}
exports.ArxivistDemoStack = ArxivistDemoStack;
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiYXJ4aXZpc3QtZGVtby1zdGFjay5qcyIsInNvdXJjZVJvb3QiOiIiLCJzb3VyY2VzIjpbImFyeGl2aXN0LWRlbW8tc3RhY2sudHMiXSwibmFtZXMiOltdLCJtYXBwaW5ncyI6Ijs7O0FBQUEsbUNBQW1DO0FBQ25DLDZDQUF5RTtBQUN6RSxtREFBbUQ7QUFDbkQseURBQXlEO0FBQ3pELHFEQUFxRDtBQUNyRCwyQ0FBMkM7QUFDM0MsMkNBQTJDO0FBQzNDLDJDQUEyQztBQUMzQyxnRUFBZ0U7QUFDaEUsNkNBQTZDO0FBQzdDLHlDQUF5QztBQUN6QywyQ0FBMkM7QUFPM0MsTUFBYSxpQkFBa0IsU0FBUSxtQkFBSztJQUMxQyxZQUFZLEtBQWdCLEVBQUUsRUFBVSxFQUFFLEtBQTZCO1FBQ3JFLEtBQUssQ0FBQyxLQUFLLEVBQUUsRUFBRSxFQUFFLEtBQUssQ0FBQyxDQUFDO1FBRXhCLE1BQU0sY0FBYyxHQUFHLElBQUksQ0FBQyxJQUFJLENBQUMsYUFBYSxDQUFDLGdCQUFnQixDQUFDLElBQUksR0FBRyxDQUFDO1FBQ3hFLE1BQU0sV0FBVyxHQUFHLElBQUksQ0FBQyxJQUFJLENBQUMsYUFBYSxDQUFDLGFBQWEsQ0FBdUIsQ0FBQztRQUNqRixNQUFNLGdCQUFnQixHQUFHLE1BQU0sQ0FBQyxJQUFJLENBQUMsSUFBSSxDQUFDLGFBQWEsQ0FBQyxrQkFBa0IsQ0FBQyxJQUFJLEVBQUUsQ0FBQyxDQUFDO1FBQ25GLE1BQU0sa0JBQWtCLEdBQUcsTUFBTSxDQUFDLElBQUksQ0FBQyxJQUFJLENBQUMsYUFBYSxDQUFDLG9CQUFvQixDQUFDLElBQUksQ0FBQyxDQUFDLENBQUM7UUFDdEYsTUFBTSxrQkFBa0IsR0FBRyxNQUFNLENBQUMsSUFBSSxDQUFDLElBQUksQ0FBQyxhQUFhLENBQUMsb0JBQW9CLENBQUMsSUFBSSxDQUFDLENBQUMsQ0FBQztRQUN0RixNQUFNLE9BQU8sR0FBRyxNQUFNLENBQUMsSUFBSSxDQUFDLElBQUksQ0FBQyxhQUFhLENBQUMsU0FBUyxDQUFDLElB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O1lBQ3BDLGFBQWEsRUFBRSxJQUFJO1lBQ25CLGNBQWMsRUFBRTtnQkFDZDtvQkFDRSxhQUFhLEVBQUUsQ0FBQztvQkFDaEIsV0FBVyxFQUFFLCtCQUErQjtpQkFDN0M7YUFDRjtTQUNGLENBQUMsQ0FBQztJQUNMLENBQUM7SUFFTyxVQUFVLENBQ2hCLEVBQVUsRUFDVixLQVFDO1FBRUQsTUFBTSxJQUFJLEdBQUcsSUFBSSxHQUFHLENBQUMscUJBQXFCLENBQUMsSUFBSSxFQUFFLEVBQUUsRUFBRTtZQUNuRCxNQUFNLEVBQUUsS0FBSyxDQUFDLE1BQU07WUFDcEIsR0FBRyxFQUFFLEtBQUssQ0FBQyxHQUFHLElBQUksR0FBRztZQUNyQixjQUFjLEVBQUUsS0FBSyxDQUFDLGNBQWMsSUFBSSxJQUFJO1NBQzdDLENBQUMsQ0FBQztRQUVILElBQUksQ0FBQyxZQUFZLENBQUMsUUFBUSxFQUFFO1lBQzFCLEtBQUssRUFBRSxHQUFHLENBQUMsY0FBYyxDQUFDLGlCQUFpQixDQUFDLEtBQUssQ0FBQyxVQUFVLEVBQUUsUUFBUSxDQUFDO1lBQ3ZFLE9BQU8sRUFBRSxLQUFLLENBQUMsT0FBTyxDQUFDLE1BQU0sR0FBRyxDQUFDLENBQUMsQ0FBQyxDQUFDLEtBQUssQ0FBQyxPQUFPLENBQUMsQ0FBQyxDQUFDLFNBQVM7WUFDN0QsT0FBTyxFQUFFLEdBQUcsQ0FBQyxVQUFVLENBQUMsT0FBTyxDQUFDO2dCQUM5QixZQUFZLEVBQUUsS0FBSyxDQUFDLE1BQU07Z0JBQzFCLFFBQVEsRUFBRSxLQUFLLENBQUMsUUFBUTthQUN6QixDQUFDO1lBQ0YsV0FBVyxFQUFFLEtBQUssQ0FBQyxXQUFXO1NBQy9CLENBQUMsQ0FBQztRQUVILE9BQU8sSUFBSSxDQUFDO0lBQ2QsQ0FBQztDQUNGO0FBNVRELDhDQTRUQyIsInNvdXJjZXNDb250ZW50IjpbImltcG9ydCAqIGFzIGNkayBmcm9tIFwiYXdzLWNkay1saWJcIjtcbmltcG9ydCB7IER1cmF0aW9uLCBSZW1vdmFsUG9saWN5LCBTdGFjaywgU3RhY2tQcm9wcyB9IGZyb20gXCJhd3MtY2RrLWxpYlwiO1xuaW1wb3J0ICogYXMgYnVkZ2V0cyBmcm9tIFwiYXdzLWNkay1saWIvYXdzLWJ1ZGdldHNcIjtcbmltcG9ydCAqIGFzIGNsb3Vkd2F0Y2ggZnJvbSBcImF3cy1jZGstbGliL2F3cy1jbG91ZHdhdGNoXCI7XG5pbXBvcnQgKiBhcyBkeW5hbW9kYiBmcm9tIFwiYXdzLWNkay1saWIvYXdzLWR5bmFtb2RiXCI7XG5pbXBvcnQgKiBhcyBlY3IgZnJvbSBcImF3cy1jZGstbGliL2F3cy1lY3JcIjtcbmltcG9ydCAqIGFzIGVjMiBmcm9tIFwiYXdzLWNkay1saWIvYXdzLWVjMlwiO1xuaW1wb3J0ICogYXMgZWNzIGZyb20gXCJhd3MtY2RrLWxpYi9hd3MtZWNzXCI7XG5pbXBvcnQgKiBhcyBlbGJ2MiBmcm9tIFwiYXdzLWNkay1saWIvYXdzLWVsYXN0aWNsb2FkYmFsYW5jaW5ndjJcIjtcbmltcG9ydCAqIGFzIGxvZ3MgZnJvbSBcImF3cy1jZGstbGliL2F3cy1sb2dzXCI7XG5pbXBvcnQgKiBhcyBzMyBmcm9tIFwiYXdzLWNkay1saWIvYXdzLXMzXCI7XG5pbXBvcnQgKiBhcyBzcXMgZnJvbSBcImF3cy1jZGstbGliL2F3cy1zcXNcIjtcbmltcG9ydCB7IENvbnN0cnVjdCB9IGZyb20gXCJjb25zdHJ1Y3RzXCI7XG5cbmludGVyZmFjZSBBcnhpdmlzdERlbW9TdGFja1Byb3BzIGV4dGVuZHMgU3RhY2tQcm9wcyB7XG4gIHByb2plY3ROYW1lOiBzdHJpbmc7XG59XG5cbmV4cG9ydCBjbGFzcyBBcnhpdmlzdERlbW9TdGFjayBleHRlbmRzIFN0YWNrIHtcbiAgY29uc3RydWN0b3Ioc2NvcGU6IENvbnN0cnVjdCwgaWQ6IHN0cmluZywgcHJvcHM6IEFyeGl2aXN0RGVtb1N0YWNrUHJvcHMpIHtcbiAgICBzdXBlcihzY29wZSwgaWQsIHByb3BzKTtcblxuICAgIGNvbnN0IGRlbW9Db3JzT3JpZ2luID0gdGhpcy5ub2RlLnRyeUdldENvbnRleHQoXCJkZW1vQ29yc09yaWdpblwiKSA/PyBcIipcIjtcbiAgICBjb25zdCBidWRnZXRFbWFpbCA9IHRoaXMubm9kZS50cnlHZXRDb250ZXh0KFwiYnVkZ2V0RW1haWxcIikgYXMgc3RyaW5nIHwgdW5kZWZpbmVkO1xuICAgIGNvbnN0IG1vbnRobHlCdWRnZXRVc2QgPSBOdW1iZXIodGhpcy5ub2RlLnRyeUdldENvbnRleHQoXCJtb250aGx5QnVkZ2V0VXNkXCIpID8/IDkwKTtcbiAgICBjb25zdCBzZWFyY2hEZXNpcmVkQ291bnQgPSBOdW1iZXIodGhpcy5ub2RlLnRyeUdldENvbnRleHQoXCJzZWFyY2hEZXNpcmVkQ291bnRcIikgPz8gMCk7XG4gICAgY29uc3QgY3Jhd2xlck1heENhcGFjaXR5ID0gTnVtYmVyKHRoaXMubm9kZS50cnlHZXRDb250ZXh0KFwiY3Jhd2xlck1heENhcGFjaXR5XCIpID8/IDQpO1xuICAgIGNvbnN0IGNyYXdsSWQgPSBTdHJpbmcodGhpcy5ub2RlLnRyeUdldENvbnRleHQoXCJjcmF3bElkXCIpID8/IFwiZGVtby01MGtcIik7XG4gICAgY29uc3QgY3Jhd2xNYXhQYWdlcyA9IE51bWJlcih0aGlzLm5vZGUudHJ5R2V0Q29udGV4dChcImNyYXdsTWF4UGFnZXNcIikgPz8gNTBfMDAwKTtcbiAgICBjb25zdCBjcmF3bE1heERlcHRoID0gTnVtYmVyKHRoaXMubm9kZS50cnlHZXRDb250ZXh0KFwiY3Jhd2xNYXhEZXB0aFwiKSA/PyA4KTtcbiAgICBjb25zdCBjcmF3bERlbGF5TXMgPSBOdW1iZXIodGhpcy5ub2RlLnRyeUdldENvbnRleHQoXCJjcmF3bERlbGF5TXNcIikgPz8gMjUwKTtcbiAgICBjb25zdCBjcmF3bEVtcHR5UmVjZWl2ZUxpbWl0ID0gTnVtYmVyKHRoaXMubm9kZS50cnlHZXRDb250ZXh0KFwiY3Jhd2xFbXB0eVJlY2VpdmVMaW1pdFwiKSA/PyAzMCk7XG4gICAgY29uc3QgbmFtZSA9IChzdWZmaXg6IHN0cmluZykgPT4gYCR7cHJvcHMucHJvamVjdE5hbWV9LSR7c3VmZml4fWA7XG5cbiAgICAvLyBDb3JwdXMgZGF0YSBpcyBpbnRlbnRpb25hbGx5IHJldGFpbmVkIHNvIGNvbXB1dGUgY2FuIGJlIGRlc3Ryb3llZCB3aXRob3V0IHJlLWNyYXdsaW5nLlxuICAgIGNvbnN0IGRhdGFCdWNrZXQgPSBuZXcgczMuQnVja2V0KHRoaXMsIFwiRGF0YUJ1Y2tldFwiLCB7XG4gICAgICBidWNrZXROYW1lOiBgJHtwcm9wcy5wcm9qZWN0TmFtZX0tZGF0YS0ke3RoaXMuYWNjb3VudH0tJHt0aGlzLnJlZ2lvbn1gLFxuICAgICAgYmxvY2tQdWJsaWNBY2Nlc3M6IHMzLkJsb2NrUHVibGljQWNjZXNzLkJMT0NLX0FMTCxcbiAgICAgIGVuY3J5cHRpb246IHMzLkJ1Y2tldEVuY3J5cHRpb24uUzNfTUFOQUdFRCxcbiAgICAgIGVuZm9yY2VTU0w6IHRydWUsXG4gICAgICByZW1vdmFsUG9saWN5OiBSZW1vdmFsUG9saWN5LlJFVEFJTixcbiAgICAgIGF1dG9EZWxldGVPYmplY3RzOiBmYWxzZSxcbiAgICAgIGxpZmVjeWNsZVJ1bGVzOiBbXG4gICAgICAgIHtcbiAgICAgICAgICBpZDogXCJleHBpcmUtb2xkLWluZGV4LWFydGlmYWN0c1wiLFxuICAgICAgICAgIHByZWZpeDogXCJpbmRleGVzL1wiLFxuICAgICAgICAgIG5vbmN1cnJlbnRWZXJzaW9uRXhwaXJhdGlvbjogRHVyYXRpb24uZGF5cygxNClcbiAgICAgICAgfVxuICAgICAgXSxcbiAgICAgIHZlcnNpb25lZDogdHJ1ZVxuICAgIH0pO1xuXG4gICAgY29uc3QgcGFnZXNUYWJsZSA9IG5ldyBkeW5hbW9kYi5UYWJsZSh0aGlzLCBcIlBhZ2VzVGFibGVcIiwge1xuICAgICAgdGFibGVOYW1lOiBuYW1lKFwicGFnZXNcIiksXG4gICAgICBwYXJ0aXRpb25LZXk6IHsgbmFtZTogXCJ1cmxfaGFzaFwiLCB0eXBlOiBkeW5hbW9kYi5BdHRyaWJ1dGVUeXBlLlNUUklORyB9LFxuICAgICAgYmlsbGluZ01vZGU6IGR5bmFtb2RiLkJpbGxpbmdNb2RlLlBBWV9QRVJfUkVRVUVTVCxcbiAgICAgIHBvaW50SW5UaW1lUmVjb3ZlcnlTcGVjaWZpY2F0aW9uOiB7XG4gICAgICAgIHBvaW50SW5UaW1lUmVjb3ZlcnlFbmFibGVkOiB0cnVlXG4gICAgICB9LFxuICAgICAgcmVtb3ZhbFBvbGljeTogUmVtb3ZhbFBvbGljeS5SRVRBSU5cbiAgICB9KTtcblxuICAgIGNvbnN0IGNyYXdsVXJsc1RhYmxlID0gbmV3IGR5bmFtb2RiLlRhYmxlKHRoaXMsIFwiQ3Jhd2xVcmxzVGFibGVcIiwge1xuICAgICAgdGFibGVOYW1lOiBuYW1lKFwiY3Jhd2wtdXJsc1wiKSxcbiAgICAgIHBhcnRpdGlvbktleTogeyBuYW1lOiBcInVybF9oYXNoXCIsIHR5cGU6IGR5bmFtb2RiLkF0dHJpYnV0ZVR5cGUuU1RSSU5HIH0sXG4gICAgICBiaWxsaW5nTW9kZTogZHluYW1vZGIuQmlsbGluZ01vZGUuUEFZX1BFUl9SRVFVRVNULFxuICAgICAgdGltZVRvTGl2ZUF0dHJpYnV0ZTogXCJleHBpcmVzX2F0XCIsXG4gICAgICByZW1vdmFsUG9saWN5OiBSZW1vdmFsUG9saWN5LlJFVEFJTlxuICAgIH0pO1xuXG4gICAgY3Jhd2xVcmxzVGFibGUuYWRkR2xvYmFsU2Vjb25kYXJ5SW5kZXgoe1xuICAgICAgaW5kZXhOYW1lOiBcImJ5LXN0YXR1c1wiLFxuICAgICAgcGFydGl0aW9uS2V5OiB7IG5hbWU6IFwic3RhdHVzXCIsIHR5cGU6IGR5bmFtb2RiLkF0dHJpYnV0ZVR5cGUuU1RSSU5HIH0sXG4gICAgICBzb3J0S2V5OiB7IG5hbWU6IFwidXBkYXRlZF9hdFwiLCB0eXBlOiBkeW5hbW9kYi5BdHRyaWJ1dGVUeXBlLlNUUklORyB9LFxuICAgICAgcHJvamVjdGlvblR5cGU6IGR5bmFtb2RiLlByb2plY3Rpb25UeXBlLkFMTFxuICAgIH0pO1xuXG4gICAgY29uc3QgZGVhZExldHRlclF1ZXVlID0gbmV3IHNxcy5RdWV1ZSh0aGlzLCBcIkNyYXdsRGVhZExldHRlclF1ZXVlXCIsIHtcbiAgICAgIHF1ZXVlTmFtZTogbmFtZShcImNyYXdsLWRscVwiKSxcbiAgICAgIHJldGVudGlvblBlcmlvZDogRHVyYXRpb24uZGF5cygxNClcbiAgICB9KTtcblxuICAgIGNvbnN0IGNyYXdsUXVldWUgPSBuZXcgc3FzLlF1ZXVlKHRoaXMsIFwiQ3Jhd2xRdWV1ZVwiLCB7XG4gICAgICBxdWV1ZU5hbWU6IG5hbWUoXCJjcmF3bC1mcm9udGllclwiKSxcbiAgICAgIHZpc2liaWxpdHlUaW1lb3V0OiBEdXJhdGlvbi5taW51dGVzKDUpLFxuICAgICAgcmV0ZW50aW9uUGVyaW9kOiBEdXJhdGlvbi5kYXlzKDQpLFxuICAgICAgZGVhZExldHRlclF1ZXVlOiB7XG4gICAgICAgIHF1ZXVlOiBkZWFkTGV0dGVyUXVldWUsXG4gICAgICAgIG1heFJlY2VpdmVDb3VudDogM1xuICAgICAgfVxuICAgIH0pO1xuXG4gICAgY29uc3QgY3Jhd2xlclJlcG9zaXRvcnkgPSB0aGlzLnJlcG9zaXRvcnkoXCJDcmF3bGVyUmVwb3NpdG9yeVwiLCBuYW1lKFwiY3Jhd2xlclwiKSk7XG4gICAgY29uc3QgaW5kZXhlclJlcG9zaXRvcnkgPSB0aGlzLnJlcG9zaXRvcnkoXCJJbmRleGVyUmVwb3NpdG9yeVwiLCBuYW1lKFwiaW5kZXhlclwiKSk7XG4gICAgY29uc3Qgc2VhcmNoUmVwb3NpdG9yeSA9IHRoaXMucmVwb3NpdG9yeShcIlNlYXJjaEFwaVJlcG9zaXRvcnlcIiwgbmFtZShcInNlYXJjaC1hcGlcIikpO1xuXG4gICAgY29uc3QgdnBjID0gbmV3IGVjMi5WcGModGhpcywgXCJWcGNcIiwge1xuICAgICAgdnBjTmFtZTogbmFtZShcInZwY1wiKSxcbiAgICAgIG5hdEdhdGV3YXlzOiAwLFxuICAgICAgbWF4QXpzOiAyLFxuICAgICAgc3VibmV0Q29uZmlndXJhdGlvbjogW1xuICAgICAgICB7XG4gICAgICAgICAgbmFtZTogXCJwdWJsaWNcIixcbiAgICAgICAgICBzdWJuZXRUeXBlOiBlYzIuU3VibmV0VHlwZS5QVUJMSUNcbiAgICAgICAgfVxuICAgICAgXVxuICAgIH0pO1xuXG4gICAgY29uc3QgY2x1c3RlciA9IG5ldyBlY3MuQ2x1c3Rlcih0aGlzLCBcIkNsdXN0ZXJcIiwge1xuICAgICAgY2x1c3Rlck5hbWU6IG5hbWUoXCJjbHVzdGVyXCIpLFxuICAgICAgdnBjLFxuICAgICAgY29udGFpbmVySW5zaWdodHNWMjogZWNzLkNvbnRhaW5lckluc2lnaHRzLkVOQUJMRURcbiAgICB9KTtcblxuICAgIGNvbnN0IGxvZ0dyb3VwID0gbmV3IGxvZ3MuTG9nR3JvdXAodGhpcywgXCJTZXJ2aWNlTG9nc1wiLCB7XG4gICAgICBsb2dHcm91cE5hbWU6IGAvYXJ4aXZpc3QvJHtwcm9wcy5wcm9qZWN0TmFtZX1gLFxuICAgICAgcmV0ZW50aW9uOiBsb2dzLlJldGVudGlvbkRheXMuT05FX1dFRUssXG4gICAgICByZW1vdmFsUG9saWN5OiBSZW1vdmFsUG9saWN5LkRFU1RST1lcbiAgICB9KTtcblxuICAgIGNvbnN0IGNyYXdsZXJUYXNrID0gdGhpcy53b3JrZXJUYXNrKFwiQ3Jhd2xlclRhc2tcIiwge1xuICAgICAgZmFtaWx5OiBuYW1lKFwiY3Jhd2xlclwiKSxcbiAgICAgIHJlcG9zaXRvcnk6IGNyYXdsZXJSZXBvc2l0b3J5LFxuICAgICAgY29tbWFuZDogW1xuICAgICAgICBcIi0tc3RvcmFnZVwiLFxuICAgICAgICBcImF3c1wiLFxuICAgICAgICBcIi0tY3Jhd2wtaWRcIixcbiAgICAgICAgY3Jhd2xJZCxcbiAgICAgICAgXCItLW1heC1wYWdlc1wiLFxuICAgICAgICBTdHJpbmcoY3Jhd2xNYXhQYWdlcyksXG4gICAgICAgIFwiLS1tYXgtZGVwdGhcIixcbiAgICAgICAgU3RyaW5nKGNyYXdsTWF4RGVwdGgpLFxuICAgICAgICBcIi0tZGVsYXktbXNcIixcbiAgICAgICAgU3RyaW5nKGNyYXdsRGVsYXlNcylcbiAgICAgIF0sXG4gICAgICBsb2dHcm91cCxcbiAgICAgIGVudmlyb25tZW50OiB7XG4gICAgICAgIEFSWElWSVNUX1NUT1JBR0VfTU9ERTogXCJhd3NcIixcbiAgICAgICAgQVJYSVZJU1RfQ1JBV0xfSUQ6IGNyYXdsSWQsXG4gICAgICAgIEFSWElWSVNUX0VNUFRZX1JFQ0VJVkVfTElNSVQ6IFN0cmluZyhjcmF3bEVtcHR5UmVjZWl2ZUxpbWl0KSxcbiAgICAgICAgQVJYSVZJU1RfREFUQV9CVUNLRVQ6IGRhdGFCdWNrZXQuYnVja2V0TmFtZSxcbiAgICAgICAgQVJYSVZJU1RfUEFHRVNfVEFCTEU6IHBhZ2VzVGFibGUudGFibGVOYW1lLFxuICAgICAgICBBUlhJVklTVF9DUkFXTF9VUkxTX1RBQkxFOiBjcmF3bFVybHNUYWJsZS50YWJsZU5hbWUsXG4gICAgICAgIEFSWElWSVNUX0NSQVdMX1FVRVVFX1VSTDogY3Jhd2xRdWV1ZS5xdWV1ZVVybFxuICAgICAgfVxuICAgIH0pO1xuXG4gICAgY29uc3QgaW5kZXhlclRhc2sgPSB0aGlzLndvcmtlclRhc2soXCJJbmRleGVyVGFza1wiLCB7XG4gICAgICBmYW1pbHk6IG5hbWUoXCJpbmRleGVyXCIpLFxuICAgICAgcmVwb3NpdG9yeTogaW5kZXhlclJlcG9zaXRvcnksXG4gICAgICBjb21tYW5kOiBbXCItLXN0b3JhZ2VcIiwgXCJhd3NcIl0sXG4gICAgICBjcHU6IDEwMjQsXG4gICAgICBtZW1vcnlMaW1pdE1pQjogNDA5NixcbiAgICAgIGxvZ0dyb3VwLFxuICAgICAgZW52aXJvbm1lbnQ6IHtcbiAgICAgICAgQVJYSVZJU1RfU1RPUkFHRV9NT0RFOiBcImF3c1wiLFxuICAgICAgICBBUlhJVklTVF9EQVRBX0JVQ0tFVDogZGF0YUJ1Y2tldC5idWNrZXROYW1lLFxuICAgICAgICBBUlhJVklTVF9QQUdFU19UQUJMRTogcGFnZXNUYWJsZS50YWJsZU5hbWUsXG4gICAgICAgIEFSWElWSVNUX0NSQVdMX1VSTFNfVEFCTEU6IGNyYXdsVXJsc1RhYmxlLnRhYmxlTmFtZSxcbiAgICAgICAgQVJYSVZJU1RfQUNUSVZFX0lOREVYX0tFWTogXCJpbmRleGVzL2FjdGl2ZS9pbmRleC5qc29uXCJcbiAgICAgIH1cbiAgICB9KTtcblxuICAgIGRhdGFCdWNrZXQuZ3JhbnRSZWFkV3JpdGUoY3Jhd2xlclRhc2sudGFza1JvbGUpO1xuICAgIGRhdGFCdWNrZXQuZ3JhbnRSZWFkV3JpdGUoaW5kZXhlclRhc2sudGFza1JvbGUpO1xuICAgIHBhZ2VzVGFibGUuZ3JhbnRSZWFkV3JpdGVEYXRhKGNyYXdsZXJUYXNrLnRhc2tSb2xlKTtcbiAgICBwYWdlc1RhYmxlLmdyYW50UmVhZFdyaXRlRGF0YShpbmRleGVyVGFzay50YXNrUm9sZSk7XG4gICAgY3Jhd2xVcmxzVGFibGUuZ3JhbnRSZWFkV3JpdGVEYXRhKGNyYXdsZXJUYXNrLnRhc2tSb2xlKTtcbiAgICBjcmF3bFVybHNUYWJsZS5ncmFudFJlYWRXcml0ZURhdGEoaW5kZXhlclRhc2sudGFza1JvbGUpO1xuICAgIGNyYXdsUXVldWUuZ3JhbnRDb25zdW1lTWVzc2FnZXMoY3Jhd2xlclRhc2sudGFza1JvbGUpO1xuICAgIGNyYXdsUXVldWUuZ3JhbnRTZW5kTWVzc2FnZXMoY3Jhd2xlclRhc2sudGFza1JvbGUpO1xuXG4gICAgY29uc3Qgc2VhcmNoVGFzayA9IG5ldyBlY3MuRmFyZ2F0ZVRhc2tEZWZpbml0aW9uKHRoaXMsIFwiU2VhcmNoVGFza1wiLCB7XG4gICAgICBmYW1pbHk6IG5hbWUoXCJzZWFyY2gtYXBpXCIpLFxuICAgICAgY3B1OiAxMDI0LFxuICAgICAgbWVtb3J5TGltaXRNaUI6IDQwOTZcbiAgICB9KTtcblxuICAgIHNlYXJjaFRhc2suYWRkQ29udGFpbmVyKFwiU2VhcmNoQXBpXCIsIHtcbiAgICAgIGltYWdlOiBlY3MuQ29udGFpbmVySW1hZ2UuZnJvbUVjclJlcG9zaXRvcnkoc2VhcmNoUmVwb3NpdG9yeSwgXCJsYXRlc3RcIiksXG4gICAgICBsb2dnaW5nOiBlY3MuTG9nRHJpdmVycy5hd3NMb2dzKHtcbiAgICAgICAgc3RyZWFtUHJlZml4OiBcInNlYXJjaC1hcGlcIixcbiAgICAgICAgbG9nR3JvdXBcbiAgICAgIH0pLFxuICAgICAgZW52aXJvbm1lbnQ6IHtcbiAgICAgICAgQVJYSVZJU1RfU1RPUkFHRV9NT0RFOiBcImF3c1wiLFxuICAgICAgICBBUlhJVklTVF9EQVRBX0JVQ0tFVDogZGF0YUJ1Y2tldC5idWNrZXROYW1lLFxuICAgICAgICBBUlhJVklTVF9BQ1RJVkVfSU5ERVhfS0VZOiBcImluZGV4ZXMvYWN0aXZlL2luZGV4Lmpzb25cIixcbiAgICAgICAgQVJYSVZJU1RfQ09SU19PUklHSU46IGRlbW9Db3JzT3JpZ2luXG4gICAgICB9LFxuICAgICAgY29tbWFuZDogW1wiLS1zdG9yYWdlXCIsIFwiYXdzXCIsIFwiLS1iaW5kXCIsIFwiMC4wLjAuMDozMDAwXCJdLFxuICAgICAgcG9ydE1hcHBpbmdzOiBbeyBjb250YWluZXJQb3J0OiAzMDAwIH1dXG4gICAgfSk7XG5cbiAgICBkYXRhQnVja2V0LmdyYW50UmVhZChzZWFyY2hUYXNrLnRhc2tSb2xlKTtcblxuICAgIGNvbnN0IHNlYXJjaExvYWRCYWxhbmNlciA9IG5ldyBlbGJ2Mi5BcHBsaWNhdGlvbkxvYWRCYWxhbmNlcih0aGlzLCBcIlNlYXJjaExvYWRCYWxhbmNlclwiLCB7XG4gICAgICBsb2FkQmFsYW5jZXJOYW1lOiBuYW1lKFwic2VhcmNoLWFwaVwiKSxcbiAgICAgIHZwYyxcbiAgICAgIGludGVybmV0RmFjaW5nOiB0cnVlXG4gICAgfSk7XG5cbiAgICBjb25zdCBzZWFyY2hMaXN0ZW5lciA9IHNlYXJjaExvYWRCYWxhbmNlci5hZGRMaXN0ZW5lcihcIlNlYXJjaEh0dHBMaXN0ZW5lclwiLCB7XG4gICAgICBwb3J0OiA4MCxcbiAgICAgIG9wZW46IHRydWVcbiAgICB9KTtcblxuICAgIC8vIEtlZXAgdGhlIHB1YmxpYyBlbmRwb2ludCBpbiBwbGFjZSB3aGlsZSBhbGxvd2luZyBkZW1vIGVudmlyb25tZW50cyB0byBpZGxlIGF0IHplcm8gdGFza3MuXG4gICAgY29uc3Qgc2VhcmNoU2VydmljZSA9IG5ldyBlY3MuRmFyZ2F0ZVNlcnZpY2UodGhpcywgXCJTZWFyY2hTZXJ2aWNlXCIsIHtcbiAgICAgIHNlcnZpY2VOYW1lOiBuYW1lKFwic2VhcmNoLWFwaVwiKSxcbiAgICAgIGNsdXN0ZXIsXG4gICAgICB0YXNrRGVmaW5pdGlvbjogc2VhcmNoVGFzayxcbiAgICAgIGRlc2lyZWRDb3VudDogc2VhcmNoRGVzaXJlZENvdW50LFxuICAgICAgY2lyY3VpdEJyZWFrZXI6IHsgcm9sbGJhY2s6IHRydWUgfSxcbiAgICAgIG1pbkhlYWx0aHlQZXJjZW50OiAxMDAsXG4gICAgICBhc3NpZ25QdWJsaWNJcDogdHJ1ZSxcbiAgICAgIHZwY1N1Ym5ldHM6IHsgc3VibmV0VHlwZTogZWMyLlN1Ym5ldFR5cGUuUFVCTElDIH1cbiAgICB9KTtcblxuICAgIGNvbnN0IHNlYXJjaFRhcmdldEdyb3VwID0gc2VhcmNoTGlzdGVuZXIuYWRkVGFyZ2V0cyhcIlNlYXJjaFRhcmdldHNcIiwge1xuICAgICAgcG9ydDogMzAwMCxcbiAgICAgIHByb3RvY29sOiBlbGJ2Mi5BcHBsaWNhdGlvblByb3RvY29sLkhUVFAsXG4gICAgICB0YXJnZXRzOiBbc2VhcmNoU2VydmljZV1cbiAgICB9KTtcblxuICAgIHNlYXJjaFRhcmdldEdyb3VwLmNvbmZpZ3VyZUhlYWx0aENoZWNrKHtcbiAgICAgIHBhdGg6IFwiL2hlYWx0aFwiLFxuICAgICAgaGVhbHRoeUh0dHBDb2RlczogXCIyMDBcIixcbiAgICAgIGludGVydmFsOiBEdXJhdGlvbi5zZWNvbmRzKDMwKVxuICAgIH0pO1xuXG4gICAgbmV3IGNsb3Vkd2F0Y2guQWxhcm0odGhpcywgXCJDcmF3bERscUFsYXJtXCIsIHtcbiAgICAgIGFsYXJtTmFtZTogbmFtZShcImNyYXdsLWRscS12aXNpYmxlXCIpLFxuICAgICAgbWV0cmljOiBkZWFkTGV0dGVyUXVldWUubWV0cmljQXBwcm94aW1hdGVOdW1iZXJPZk1lc3NhZ2VzVmlzaWJsZSgpLFxuICAgICAgdGhyZXNob2xkOiAxLFxuICAgICAgZXZhbHVhdGlvblBlcmlvZHM6IDFcbiAgICB9KTtcblxuICAgIGlmIChidWRnZXRFbWFpbCAmJiBidWRnZXRFbWFpbC50cmltKCkgIT09IFwiXCIpIHtcbiAgICAgIG5ldyBidWRnZXRzLkNmbkJ1ZGdldCh0aGlzLCBcIk1vbnRobHlCdWRnZXRcIiwge1xuICAgICAgICBidWRnZXQ6IHtcbiAgICAgICAgICBidWRnZXROYW1lOiBuYW1lKFwibW9udGhseS1kZW1vLWJ1ZGdldFwiKSxcbiAgICAgICAgICBidWRnZXRMaW1pdDoge1xuICAgICAgICAgICAgYW1vdW50OiBtb250aGx5QnVkZ2V0VXNkLFxuICAgICAgICAgICAgdW5pdDogXCJVU0RcIlxuICAgICAgICAgIH0sXG4gICAgICAgICAgdGltZVVuaXQ6IFwiTU9OVEhMWVwiLFxuICAgICAgICAgIGJ1ZGdldFR5cGU6IFwiQ09TVFwiXG4gICAgICAgIH0sXG4gICAgICAgIG5vdGlmaWNhdGlvbnNXaXRoU3Vic2NyaWJlcnM6IFtcbiAgICAgICAgICB7XG4gICAgICAgICAgICBub3RpZmljYXRpb246IHtcbiAgICAgICAgICAgICAgbm90aWZpY2F0aW9uVHlwZTogXCJBQ1RVQUxcIixcbiAgICAgICAgICAgICAgY29tcGFyaXNvbk9wZXJhdG9yOiBcIkdSRUFURVJfVEhBTlwiLFxuICAgICAgICAgICAgICB0aHJlc2hvbGQ6IDgwLFxuICAgICAgICAgICAgICB0aHJlc2hvbGRUeXBlOiBcIlBFUkNFTlRBR0VcIlxuICAgICAgICAgICAgfSxcbiAgICAgICAgICAgIHN1YnNjcmliZXJzOiBbXG4gICAgICAgICAgICAgIHtcbiAgICAgICAgICAgICAgICBzdWJzY3JpcHRpb25UeXBlOiBcIkVNQUlMXCIsXG4gICAgICAgICAgICAgICAgYWRkcmVzczogYnVkZ2V0RW1haWxcbiAgICAgICAgICAgICAgfVxuICAgICAgICAgICAgXVxuICAgICAgICAgIH1cbiAgICAgICAgXVxuICAgICAgfSk7XG4gICAgfVxuXG4gICAgbmV3IGNkay5DZm5PdXRwdXQodGhpcywgXCJEYXRhQnVja2V0TmFtZVwiLCB7IHZhbHVlOiBkYXRhQnVja2V0LmJ1Y2tldE5hbWUgfSk7XG4gICAgbmV3IGNkay5DZm5PdXRwdXQodGhpcywgXCJQYWdlc1RhYmxlTmFtZVwiLCB7IHZhbHVlOiBwYWdlc1RhYmxlLnRhYmxlTmFtZSB9KTtcbiAgICBuZXcgY2RrLkNmbk91dHB1dCh0aGlzLCBcIkNyYXdsVXJsc1RhYmxlTmFtZVwiLCB7IHZhbHVlOiBjcmF3bFVybHNUYWJsZS50YWJsZU5hbWUgfSk7XG4gICAgbmV3IGNkay5DZm5PdXRwdXQodGhpcywgXCJDcmF3bFF1ZXVlVXJsXCIsIHsgdmFsdWU6IGNyYXdsUXVldWUucXVldWVVcmwgfSk7XG4gICAgbmV3IGNkay5DZm5PdXRwdXQodGhpcywgXCJTZWFyY2hBcGlVcmxcIiwge1xuICAgICAgdmFsdWU6IGBodHRwOi8vJHtzZWFyY2hMb2FkQmFsYW5jZXIubG9hZEJhbGFuY2VyRG5zTmFtZX1gXG4gICAgfSk7XG4gICAgbmV3IGNkay5DZm5PdXRwdXQodGhpcywgXCJDcmF3bGVySW1hZ2VCdWlsZFwiLCB7XG4gICAgICB2YWx1ZTogYGRvY2tlciBidWlsZCAtLWJ1aWxkLWFyZyBCSU49YXJ4aXZpc3QtY3Jhd2xlciAtdCAke2NyYXdsZXJSZXBvc2l0b3J5LnJlcG9zaXRvcnlVcml9OmxhdGVzdCAuYFxuICAgIH0pO1xuICAgIG5ldyBjZGsuQ2ZuT3V0cHV0KHRoaXMsIFwiSW5kZXhlckltYWdlQnVpbGRcIiwge1xuICAgICAgdmFsdWU6IGBkb2NrZXIgYnVpbGQgLS1idWlsZC1hcmcgQklOPWFyeGl2aXN0LWluZGV4ZXIgLXQgJHtpbmRleGVyUmVwb3NpdG9yeS5yZXBvc2l0b3J5VXJpfTpsYXRlc3QgLmBcbiAgICB9KTtcbiAgICBuZXcgY2RrLkNmbk91dHB1dCh0aGlzLCBcIlNlYXJjaEFwaUltYWdlQnVpbGRcIiwge1xuICAgICAgdmFsdWU6IGBkb2NrZXIgYnVpbGQgLS1idWlsZC1hcmcgQklOPWFyeGl2aXN0LXNlYXJjaC1hcGkgLXQgJHtzZWFyY2hSZXBvc2l0b3J5LnJlcG9zaXRvcnlVcml9OmxhdGVzdCAuYFxuICAgIH0pO1xuICAgIG5ldyBjZGsuQ2ZuT3V0cHV0KHRoaXMsIFwiQ3Jhd2xlck1heENhcGFjaXR5XCIsIHsgdmFsdWU6IFN0cmluZyhjcmF3bGVyTWF4Q2FwYWNpdHkpIH0pO1xuICAgIG5ldyBjZGsuQ2ZuT3V0cHV0KHRoaXMsIFwiQ3Jhd2xJZFwiLCB7IHZhbHVlOiBjcmF3bElkIH0pO1xuICAgIG5ldyBjZGsuQ2ZuT3V0cHV0KHRoaXMsIFwiQ3Jhd2xNYXhQYWdlc1wiLCB7IHZhbHVlOiBTdHJpbmcoY3Jhd2xNYXhQYWdlcykgfSk7XG4gIH1cblxuICBwcml2YXRlIHJlcG9zaXRvcnkoaWQ6IHN0cmluZywgcmVwb3NpdG9yeU5hbWU6IHN0cmluZyk6IGVjci5SZXBvc2l0b3J5IHtcbiAgICByZXR1cm4gbmV3IGVjci5SZXBvc2l0b3J5KHRoaXMsIGlkLCB7XG4gICAgICByZXBvc2l0b3J5TmFtZSxcbiAgICAgIGltYWdlU2Nhbk9uUHVzaDogdHJ1ZSxcbiAgICAgIHJlbW92YWxQb2xpY3k6IFJlbW92YWxQb2xpY3kuREVTVFJPWSxcbiAgICAgIGVtcHR5T25EZWxldGU6IHRydWUsXG4gICAgICBsaWZlY3ljbGVSdWxlczogW1xuICAgICAgICB7XG4gICAgICAgICAgbWF4SW1hZ2VDb3VudDogNSxcbiAgICAgICAgICBkZXNjcmlwdGlvbjogXCJLZWVwIG9ubHkgcmVjZW50IGRlbW8gaW1hZ2VzLlwiXG4gICAgICAgIH1cbiAgICAgIF1cbiAgICB9KTtcbiAgfVxuXG4gIHByaXZhdGUgd29ya2VyVGFzayhcbiAgICBpZDogc3RyaW5nLFxuICAgIHByb3BzOiB7XG4gICAgICBmYW1pbHk6IHN0cmluZztcbiAgICAgIHJlcG9zaXRvcnk6IGVjci5JUmVwb3NpdG9yeTtcbiAgICAgIGNvbW1hbmQ6IHN0cmluZ1tdO1xuICAgICAgY3B1PzogbnVtYmVyO1xuICAgICAgbWVtb3J5TGltaXRNaUI/OiBudW1iZXI7XG4gICAgICBsb2dHcm91cDogbG9ncy5JTG9nR3JvdXA7XG4gICAgICBlbnZpcm9ubWVudDogUmVjb3JkPHN0cmluZywgc3RyaW5nPjtcbiAgICB9XG4gICk6IGVjcy5GYXJnYXRlVGFza0RlZmluaXRpb24ge1xuICAgIGNvbnN0IHRhc2sgPSBuZXcgZWNzLkZhcmdhdGVUYXNrRGVmaW5pdGlvbih0aGlzLCBpZCwge1xuICAgICAgZmFtaWx5OiBwcm9wcy5mYW1pbHksXG4gICAgICBjcHU6IHByb3BzLmNwdSA/PyA1MTIsXG4gICAgICBtZW1vcnlMaW1pdE1pQjogcHJvcHMubWVtb3J5TGltaXRNaUIgPz8gMTAyNFxuICAgIH0pO1xuXG4gICAgdGFzay5hZGRDb250YWluZXIoXCJXb3JrZXJcIiwge1xuICAgICAgaW1hZ2U6IGVjcy5Db250YWluZXJJbWFnZS5mcm9tRWNyUmVwb3NpdG9yeShwcm9wcy5yZXBvc2l0b3J5LCBcImxhdGVzdFwiKSxcbiAgICAgIGNvbW1hbmQ6IHByb3BzLmNvbW1hbmQubGVuZ3RoID4gMCA/IHByb3BzLmNvbW1hbmQgOiB1bmRlZmluZWQsXG4gICAgICBsb2dnaW5nOiBlY3MuTG9nRHJpdmVycy5hd3NMb2dzKHtcbiAgICAgICAgc3RyZWFtUHJlZml4OiBwcm9wcy5mYW1pbHksXG4gICAgICAgIGxvZ0dyb3VwOiBwcm9wcy5sb2dHcm91cFxuICAgICAgfSksXG4gICAgICBlbnZpcm9ubWVudDogcHJvcHMuZW52aXJvbm1lbnRcbiAgICB9KTtcblxuICAgIHJldHVybiB0YXNrO1xuICB9XG59XG4iXX0=