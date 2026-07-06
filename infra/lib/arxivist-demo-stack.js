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
const apigatewayv2 = require("aws-cdk-lib/aws-apigatewayv2");
const integrations = require("aws-cdk-lib/aws-apigatewayv2-integrations");
const lambda = require("aws-cdk-lib/aws-lambda");
const logs = require("aws-cdk-lib/aws-logs");
const s3 = require("aws-cdk-lib/aws-s3");
const secretsmanager = require("aws-cdk-lib/aws-secretsmanager");
const sqs = require("aws-cdk-lib/aws-sqs");
const path = require("path");
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
        const agentTimeoutSeconds = Number(this.node.tryGetContext("agentTimeoutSeconds") ?? 60);
        const openAiApiKeySecretName = String(this.node.tryGetContext("openAiApiKeySecretName") ?? `${props.projectName}/openai-api-key`);
        const name = (suffix) => `${props.projectName}-${suffix}`;
        this.addGlobalCostTags(props.projectName);
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
        this.addServiceTags(dataBucket, "storage", "crawl-snapshots");
        const pagesTable = new dynamodb.Table(this, "PagesTable", {
            tableName: name("pages"),
            partitionKey: { name: "url_hash", type: dynamodb.AttributeType.STRING },
            billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
            pointInTimeRecoverySpecification: {
                pointInTimeRecoveryEnabled: true
            },
            removalPolicy: aws_cdk_lib_1.RemovalPolicy.RETAIN
        });
        this.addServiceTags(pagesTable, "metadata", "pages-table");
        const crawlUrlsTable = new dynamodb.Table(this, "CrawlUrlsTable", {
            tableName: name("crawl-urls"),
            partitionKey: { name: "url_hash", type: dynamodb.AttributeType.STRING },
            billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
            timeToLiveAttribute: "expires_at",
            removalPolicy: aws_cdk_lib_1.RemovalPolicy.RETAIN
        });
        this.addServiceTags(crawlUrlsTable, "metadata", "crawl-url-dedupe");
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
        this.addServiceTags(deadLetterQueue, "queue", "crawl-dead-letter");
        const crawlQueue = new sqs.Queue(this, "CrawlQueue", {
            queueName: name("crawl-frontier"),
            visibilityTimeout: aws_cdk_lib_1.Duration.minutes(5),
            retentionPeriod: aws_cdk_lib_1.Duration.days(4),
            deadLetterQueue: {
                queue: deadLetterQueue,
                maxReceiveCount: 3
            }
        });
        this.addServiceTags(crawlQueue, "queue", "crawl-frontier");
        const crawlerRepository = this.repository("CrawlerRepository", name("crawler"));
        const indexerRepository = this.repository("IndexerRepository", name("indexer"));
        const searchRepository = this.repository("SearchApiRepository", name("search-api"));
        this.addServiceTags(crawlerRepository, "crawler", "container-image");
        this.addServiceTags(indexerRepository, "indexer", "container-image");
        this.addServiceTags(searchRepository, "search-api", "container-image");
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
        this.addServiceTags(vpc, "network", "public-vpc");
        const cluster = new ecs.Cluster(this, "Cluster", {
            clusterName: name("cluster"),
            vpc,
            containerInsightsV2: ecs.ContainerInsights.ENABLED
        });
        this.addServiceTags(cluster, "compute", "ecs-cluster");
        const logGroup = new logs.LogGroup(this, "ServiceLogs", {
            logGroupName: `/arxivist/${props.projectName}`,
            retention: logs.RetentionDays.ONE_WEEK,
            removalPolicy: aws_cdk_lib_1.RemovalPolicy.DESTROY
        });
        this.addServiceTags(logGroup, "observability", "service-logs");
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
        this.addServiceTags(crawlerTask, "crawler", "worker-task");
        const indexerTask = this.workerTask("IndexerTask", {
            family: name("indexer"),
            repository: indexerRepository,
            command: ["--storage", "aws"],
            cpu: 1024,
            memoryLimitMiB: 8192,
            logGroup,
            environment: {
                ARXIVIST_STORAGE_MODE: "aws",
                ARXIVIST_DATA_BUCKET: dataBucket.bucketName,
                ARXIVIST_PAGES_TABLE: pagesTable.tableName,
                ARXIVIST_CRAWL_URLS_TABLE: crawlUrlsTable.tableName,
                ARXIVIST_ACTIVE_INDEX_KEY: "indexes/active/manifest.json"
            }
        });
        this.addServiceTags(indexerTask, "indexer", "worker-task");
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
        this.addServiceTags(searchTask, "search-api", "api-task");
        searchTask.addContainer("SearchApi", {
            image: ecs.ContainerImage.fromEcrRepository(searchRepository, "latest"),
            logging: ecs.LogDrivers.awsLogs({
                streamPrefix: "search-api",
                logGroup
            }),
            environment: {
                ARXIVIST_STORAGE_MODE: "aws",
                ARXIVIST_DATA_BUCKET: dataBucket.bucketName,
                ARXIVIST_ACTIVE_INDEX_KEY: "indexes/active/manifest.json",
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
        this.addServiceTags(searchLoadBalancer, "search-api", "public-load-balancer");
        const searchListener = searchLoadBalancer.addListener("SearchHttpListener", {
            port: 80,
            open: true
        });
        this.addServiceTags(searchListener, "search-api", "http-listener");
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
        this.addServiceTags(searchService, "search-api", "api-service");
        const searchTargetGroup = searchListener.addTargets("SearchTargets", {
            port: 3000,
            protocol: elbv2.ApplicationProtocol.HTTP,
            targets: [searchService]
        });
        this.addServiceTags(searchTargetGroup, "search-api", "api-target-group");
        searchTargetGroup.configureHealthCheck({
            path: "/health",
            healthyHttpCodes: "200",
            interval: aws_cdk_lib_1.Duration.seconds(30)
        });
        const openAiApiKeySecret = secretsmanager.Secret.fromSecretNameV2(this, "OpenAiApiKeySecret", openAiApiKeySecretName);
        // The agent remains a separate boundary so the learning graph can evolve
        // without coupling LLM behavior to the traditional Rust search API.
        const agentFunction = new lambda.DockerImageFunction(this, "AgentFunction", {
            functionName: name("agent-api"),
            code: lambda.DockerImageCode.fromImageAsset(path.join(__dirname, "../../arxivist-agent")),
            memorySize: 1024,
            timeout: aws_cdk_lib_1.Duration.seconds(agentTimeoutSeconds),
            environment: {
                ARXIVIST_STORAGE_MODE: "aws",
                ARXIVIST_DATA_BUCKET: dataBucket.bucketName,
                ARXIVIST_PAGES_TABLE: pagesTable.tableName,
                ARXIVIST_SEARCH_API_BASE_URL: `http://${searchLoadBalancer.loadBalancerDnsName}`,
                OPENAI_API_KEY_SECRET_NAME: openAiApiKeySecretName
            }
        });
        this.addServiceTags(agentFunction, "agent-api", "lambda-function");
        openAiApiKeySecret.grantRead(agentFunction);
        dataBucket.grantRead(agentFunction);
        pagesTable.grantReadData(agentFunction);
        const agentApi = new apigatewayv2.HttpApi(this, "AgentHttpApi", {
            apiName: name("agent-api"),
            corsPreflight: {
                allowHeaders: ["content-type"],
                allowMethods: [
                    apigatewayv2.CorsHttpMethod.GET,
                    apigatewayv2.CorsHttpMethod.POST,
                    apigatewayv2.CorsHttpMethod.OPTIONS
                ],
                allowOrigins: [demoCorsOrigin]
            }
        });
        this.addServiceTags(agentApi, "agent-api", "http-api");
        const agentIntegration = new integrations.HttpLambdaIntegration("AgentLambdaIntegration", agentFunction);
        agentApi.addRoutes({
            path: "/agent/health",
            methods: [apigatewayv2.HttpMethod.GET],
            integration: agentIntegration
        });
        agentApi.addRoutes({
            path: "/agent/search",
            methods: [apigatewayv2.HttpMethod.POST],
            integration: agentIntegration
        });
        const crawlDlqAlarm = new cloudwatch.Alarm(this, "CrawlDlqAlarm", {
            alarmName: name("crawl-dlq-visible"),
            metric: deadLetterQueue.metricApproximateNumberOfMessagesVisible(),
            threshold: 1,
            evaluationPeriods: 1
        });
        this.addServiceTags(crawlDlqAlarm, "observability", "crawl-dlq-alarm");
        if (budgetEmail && budgetEmail.trim() !== "") {
            const monthlyBudget = new budgets.CfnBudget(this, "MonthlyBudget", {
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
            this.addServiceTags(monthlyBudget, "observability", "monthly-budget");
        }
        new cdk.CfnOutput(this, "DataBucketName", { value: dataBucket.bucketName });
        new cdk.CfnOutput(this, "PagesTableName", { value: pagesTable.tableName });
        new cdk.CfnOutput(this, "CrawlUrlsTableName", { value: crawlUrlsTable.tableName });
        new cdk.CfnOutput(this, "CrawlQueueUrl", { value: crawlQueue.queueUrl });
        new cdk.CfnOutput(this, "SearchApiUrl", {
            value: `http://${searchLoadBalancer.loadBalancerDnsName}`
        });
        new cdk.CfnOutput(this, "AgentApiUrl", { value: agentApi.apiEndpoint });
        new cdk.CfnOutput(this, "OpenAiApiKeySecretName", { value: openAiApiKeySecretName });
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
    addGlobalCostTags(projectName) {
        cdk.Tags.of(this).add("Project", projectName);
        cdk.Tags.of(this).add("Environment", "demo");
        cdk.Tags.of(this).add("ManagedBy", "cdk");
        cdk.Tags.of(this).add("Owner", "kevius");
        cdk.Tags.of(this).add("CostCenter", "arxivist-demo");
    }
    addServiceTags(resource, service, component) {
        cdk.Tags.of(resource).add("Service", service);
        cdk.Tags.of(resource).add("Component", component);
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
//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiYXJ4aXZpc3QtZGVtby1zdGFjay5qcyIsInNvdXJjZVJvb3QiOiIiLCJzb3VyY2VzIjpbImFyeGl2aXN0LWRlbW8tc3RhY2sudHMiXSwibmFtZXMiOltdLCJtYXBwaW5ncyI6Ijs7O0FBQUEsbUNBQW1DO0FBQ25DLDZDQUF5RTtBQUN6RSxtREFBbUQ7QUFDbkQseURBQXlEO0FBQ3pELHFEQUFxRDtBQUNyRCwyQ0FBMkM7QUFDM0MsMkNBQTJDO0FBQzNDLDJDQUEyQztBQUMzQyxnRUFBZ0U7QUFDaEUsNkRBQTZEO0FBQzdELDBFQUEwRTtBQUMxRSxpREFBaUQ7QUFDakQsNkNBQTZDO0FBQzdDLHlDQUF5QztBQUN6QyxpRUFBaUU7QUFDakUsMkNBQTJDO0FBRTNDLDZCQUE2QjtBQU03QixNQUFhLGlCQUFrQixTQUFRLG1CQUFLO0lBQzFDLFlBQVksS0FBZ0IsRUFBRSxFQUFVLEVBQUUsS0FBNkI7UUFDckUsS0FBSyxDQUFDLEtBQUssRUFBRSxFQUFFLEVBQUUsS0FBSyxDQUFDLENBQUM7UUFFeEIsTUFBTSxjQUFjLEdBQUcsSUFBSSxDQUFDLElBQUksQ0FBQyxhQUFhLENBQUMsZ0JBQWdCLENBQUMsSUFBSSxHQUFHLENBQUM7UUFDeEUsTUFBTSxXQUFXLEdBQUcsSUFBSSxDQUFDLElBQUksQ0FBQyxhQUFhLENBQUMsYUFBYSxDQUF1QixDQUFDO1FBQ2pGLE1BQU0sZ0JBQWdCLEdBQUcsTUFBTSxDQUFDLElBQUksQ0FBQyxJQUFJLENBQUMsYUFBYSxDQUFDLGtCQUFrQixDQUFDLElBQUksRUFBRSxDQUFDLENBQUM7UUFDbkYsTUFBTSxrQkFBa0IsR0FBRyxNQUFNLENBQUMsSUFBSSxDQUFDLElBQUksQ0FBQyxhQUFhLENBQUMsb0JBQW9CLENBQUMsSUFBSSxDQUFDLENBQUMsQ0FBQztRQUN0RixNQUFNLGtCQUFrQixHQUFHLE1BQU0sQ0FBQyxJQUFJLENBQUMsSUFBSSxDQUFDLGFBQWEsQ0FBQyxvQkFBb0IsQ0FBQyxJQUFJLENBQUMsQ0FBQyxDQUFDO1FBQ3RGLE1B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QU0sR0FBRyxDQUFDLENBQUMsQ0FBQyxDQUFDLEtBQUssQ0FBQyxPQUFPLENBQUMsQ0FBQyxDQUFDLFNBQVM7WUFDN0QsT0FBTyxFQUFFLEdBQUcsQ0FBQyxVQUFVLENBQUMsT0FBTyxDQUFDO2dCQUM5QixZQUFZLEVBQUUsS0FBSyxDQUFDLE1BQU07Z0JBQzFCLFFBQVEsRUFBRSxLQUFLLENBQUMsUUFBUTthQUN6QixDQUFDO1lBQ0YsV0FBVyxFQUFFLEtBQUssQ0FBQyxXQUFXO1NBQy9CLENBQUMsQ0FBQztRQUVILE9BQU8sSUFBSSxDQUFDO0lBQ2QsQ0FBQztDQUNGO0FBOVpELDhDQThaQyIsInNvdXJjZXNDb250ZW50IjpbImltcG9ydCAqIGFzIGNkayBmcm9tIFwiYXdzLWNkay1saWJcIjtcbmltcG9ydCB7IER1cmF0aW9uLCBSZW1vdmFsUG9saWN5LCBTdGFjaywgU3RhY2tQcm9wcyB9IGZyb20gXCJhd3MtY2RrLWxpYlwiO1xuaW1wb3J0ICogYXMgYnVkZ2V0cyBmcm9tIFwiYXdzLWNkay1saWIvYXdzLWJ1ZGdldHNcIjtcbmltcG9ydCAqIGFzIGNsb3Vkd2F0Y2ggZnJvbSBcImF3cy1jZGstbGliL2F3cy1jbG91ZHdhdGNoXCI7XG5pbXBvcnQgKiBhcyBkeW5hbW9kYiBmcm9tIFwiYXdzLWNkay1saWIvYXdzLWR5bmFtb2RiXCI7XG5pbXBvcnQgKiBhcyBlY3IgZnJvbSBcImF3cy1jZGstbGliL2F3cy1lY3JcIjtcbmltcG9ydCAqIGFzIGVjMiBmcm9tIFwiYXdzLWNkay1saWIvYXdzLWVjMlwiO1xuaW1wb3J0ICogYXMgZWNzIGZyb20gXCJhd3MtY2RrLWxpYi9hd3MtZWNzXCI7XG5pbXBvcnQgKiBhcyBlbGJ2MiBmcm9tIFwiYXdzLWNkay1saWIvYXdzLWVsYXN0aWNsb2FkYmFsYW5jaW5ndjJcIjtcbmltcG9ydCAqIGFzIGFwaWdhdGV3YXl2MiBmcm9tIFwiYXdzLWNkay1saWIvYXdzLWFwaWdhdGV3YXl2MlwiO1xuaW1wb3J0ICogYXMgaW50ZWdyYXRpb25zIGZyb20gXCJhd3MtY2RrLWxpYi9hd3MtYXBpZ2F0ZXdheXYyLWludGVncmF0aW9uc1wiO1xuaW1wb3J0ICogYXMgbGFtYmRhIGZyb20gXCJhd3MtY2RrLWxpYi9hd3MtbGFtYmRhXCI7XG5pbXBvcnQgKiBhcyBsb2dzIGZyb20gXCJhd3MtY2RrLWxpYi9hd3MtbG9nc1wiO1xuaW1wb3J0ICogYXMgczMgZnJvbSBcImF3cy1jZGstbGliL2F3cy1zM1wiO1xuaW1wb3J0ICogYXMgc2VjcmV0c21hbmFnZXIgZnJvbSBcImF3cy1jZGstbGliL2F3cy1zZWNyZXRzbWFuYWdlclwiO1xuaW1wb3J0ICogYXMgc3FzIGZyb20gXCJhd3MtY2RrLWxpYi9hd3Mtc3FzXCI7XG5pbXBvcnQgeyBDb25zdHJ1Y3QgfSBmcm9tIFwiY29uc3RydWN0c1wiO1xuaW1wb3J0ICogYXMgcGF0aCBmcm9tIFwicGF0aFwiO1xuXG5pbnRlcmZhY2UgQXJ4aXZpc3REZW1vU3RhY2tQcm9wcyBleHRlbmRzIFN0YWNrUHJvcHMge1xuICBwcm9qZWN0TmFtZTogc3RyaW5nO1xufVxuXG5leHBvcnQgY2xhc3MgQXJ4aXZpc3REZW1vU3RhY2sgZXh0ZW5kcyBTdGFjayB7XG4gIGNvbnN0cnVjdG9yKHNjb3BlOiBDb25zdHJ1Y3QsIGlkOiBzdHJpbmcsIHByb3BzOiBBcnhpdmlzdERlbW9TdGFja1Byb3BzKSB7XG4gICAgc3VwZXIoc2NvcGUsIGlkLCBwcm9wcyk7XG5cbiAgICBjb25zdCBkZW1vQ29yc09yaWdpbiA9IHRoaXMubm9kZS50cnlHZXRDb250ZXh0KFwiZGVtb0NvcnNPcmlnaW5cIikgPz8gXCIqXCI7XG4gICAgY29uc3QgYnVkZ2V0RW1haWwgPSB0aGlzLm5vZGUudHJ5R2V0Q29udGV4dChcImJ1ZGdldEVtYWlsXCIpIGFzIHN0cmluZyB8IHVuZGVmaW5lZDtcbiAgICBjb25zdCBtb250aGx5QnVkZ2V0VXNkID0gTnVtYmVyKHRoaXMubm9kZS50cnlHZXRDb250ZXh0KFwibW9udGhseUJ1ZGdldFVzZFwiKSA/PyA5MCk7XG4gICAgY29uc3Qgc2VhcmNoRGVzaXJlZENvdW50ID0gTnVtYmVyKHRoaXMubm9kZS50cnlHZXRDb250ZXh0KFwic2VhcmNoRGVzaXJlZENvdW50XCIpID8/IDApO1xuICAgIGNvbnN0IGNyYXdsZXJNYXhDYXBhY2l0eSA9IE51bWJlcih0aGlzLm5vZGUudHJ5R2V0Q29udGV4dChcImNyYXdsZXJNYXhDYXBhY2l0eVwiKSA/PyA0KTtcbiAgICBjb25zdCBjcmF3bElkID0gU3RyaW5nKHRoaXMubm9kZS50cnlHZXRDb250ZXh0KFwiY3Jhd2xJZFwiKSA/PyBcImRlbW8tNTBrXCIpO1xuICAgIGNvbnN0IGNyYXdsTWF4UGFnZXMgPSBOdW1iZXIodGhpcy5ub2RlLnRyeUdldENvbnRleHQoXCJjcmF3bE1heFBhZ2VzXCIpID8/IDUwXzAwMCk7XG4gICAgY29uc3QgY3Jhd2xNYXhEZXB0aCA9IE51bWJlcih0aGlzLm5vZGUudHJ5R2V0Q29udGV4dChcImNyYXdsTWF4RGVwdGhcIikgPz8gOCk7XG4gICAgY29uc3QgY3Jhd2xEZWxheU1zID0gTnVtYmVyKHRoaXMubm9kZS50cnlHZXRDb250ZXh0KFwiY3Jhd2xEZWxheU1zXCIpID8/IDI1MCk7XG4gICAgY29uc3QgY3Jhd2xFbXB0eVJlY2VpdmVMaW1pdCA9IE51bWJlcih0aGlzLm5vZGUudHJ5R2V0Q29udGV4dChcImNyYXdsRW1wdHlSZWNlaXZlTGltaXRcIikgPz8gMzApO1xuICAgIGNvbnN0IGFnZW50VGltZW91dFNlY29uZHMgPSBOdW1iZXIodGhpcy5ub2RlLnRyeUdldENvbnRleHQoXCJhZ2VudFRpbWVvdXRTZWNvbmRzXCIpID8/IDYwKTtcbiAgICBjb25zdCBvcGVuQWlBcGlLZXlTZWNyZXROYW1lID0gU3RyaW5nKFxuICAgICAgdGhpcy5ub2RlLnRyeUdldENvbnRleHQoXCJvcGVuQWlBcGlLZXlTZWNyZXROYW1lXCIpID8/IGAke3Byb3BzLnByb2plY3ROYW1lfS9vcGVuYWktYXBpLWtleWBcbiAgICApO1xuICAgIGNvbnN0IG5hbWUgPSAoc3VmZml4OiBzdHJpbmcpID0+IGAke3Byb3BzLnByb2plY3ROYW1lfS0ke3N1ZmZpeH1gO1xuXG4gICAgdGhpcy5hZGRHbG9iYWxDb3N0VGFncyhwcm9wcy5wcm9qZWN0TmFtZSk7XG5cbiAgICAvLyBDb3JwdXMgZGF0YSBpcyBpbnRlbnRpb25hbGx5IHJldGFpbmVkIHNvIGNvbXB1dGUgY2FuIGJlIGRlc3Ryb3llZCB3aXRob3V0IHJlLWNyYXdsaW5nLlxuICAgIGNvbnN0IGRhdGFCdWNrZXQgPSBuZXcgczMuQnVja2V0KHRoaXMsIFwiRGF0YUJ1Y2tldFwiLCB7XG4gICAgICBidWNrZXROYW1lOiBgJHtwcm9wcy5wcm9qZWN0TmFtZX0tZGF0YS0ke3RoaXMuYWNjb3VudH0tJHt0aGlzLnJlZ2lvbn1gLFxuICAgICAgYmxvY2tQdWJsaWNBY2Nlc3M6IHMzLkJsb2NrUHVibGljQWNjZXNzLkJMT0NLX0FMTCxcbiAgICAgIGVuY3J5cHRpb246IHMzLkJ1Y2tldEVuY3J5cHRpb24uUzNfTUFOQUdFRCxcbiAgICAgIGVuZm9yY2VTU0w6IHRydWUsXG4gICAgICByZW1vdmFsUG9saWN5OiBSZW1vdmFsUG9saWN5LlJFVEFJTixcbiAgICAgIGF1dG9EZWxldGVPYmplY3RzOiBmYWxzZSxcbiAgICAgIGxpZmVjeWNsZVJ1bGVzOiBbXG4gICAgICAgIHtcbiAgICAgICAgICBpZDogXCJleHBpcmUtb2xkLWluZGV4LWFydGlmYWN0c1wiLFxuICAgICAgICAgIHByZWZpeDogXCJpbmRleGVzL1wiLFxuICAgICAgICAgIG5vbmN1cnJlbnRWZXJzaW9uRXhwaXJhdGlvbjogRHVyYXRpb24uZGF5cygxNClcbiAgICAgICAgfVxuICAgICAgXSxcbiAgICAgIHZlcnNpb25lZDogdHJ1ZVxuICAgIH0pO1xuICAgIHRoaXMuYWRkU2VydmljZVRhZ3MoZGF0YUJ1Y2tldCwgXCJzdG9yYWdlXCIsIFwiY3Jhd2wtc25hcHNob3RzXCIpO1xuXG4gICAgY29uc3QgcGFnZXNUYWJsZSA9IG5ldyBkeW5hbW9kYi5UYWJsZSh0aGlzLCBcIlBhZ2VzVGFibGVcIiwge1xuICAgICAgdGFibGVOYW1lOiBuYW1lKFwicGFnZXNcIiksXG4gICAgICBwYXJ0aXRpb25LZXk6IHsgbmFtZTogXCJ1cmxfaGFzaFwiLCB0eXBlOiBkeW5hbW9kYi5BdHRyaWJ1dGVUeXBlLlNUUklORyB9LFxuICAgICAgYmlsbGluZ01vZGU6IGR5bmFtb2RiLkJpbGxpbmdNb2RlLlBBWV9QRVJfUkVRVUVTVCxcbiAgICAgIHBvaW50SW5UaW1lUmVjb3ZlcnlTcGVjaWZpY2F0aW9uOiB7XG4gICAgICAgIHBvaW50SW5UaW1lUmVjb3ZlcnlFbmFibGVkOiB0cnVlXG4gICAgICB9LFxuICAgICAgcmVtb3ZhbFBvbGljeTogUmVtb3ZhbFBvbGljeS5SRVRBSU5cbiAgICB9KTtcbiAgICB0aGlzLmFkZFNlcnZpY2VUYWdzKHBhZ2VzVGFibGUsIFwibWV0YWRhdGFcIiwgXCJwYWdlcy10YWJsZVwiKTtcblxuICAgIGNvbnN0IGNyYXdsVXJsc1RhYmxlID0gbmV3IGR5bmFtb2RiLlRhYmxlKHRoaXMsIFwiQ3Jhd2xVcmxzVGFibGVcIiwge1xuICAgICAgdGFibGVOYW1lOiBuYW1lKFwiY3Jhd2wtdXJsc1wiKSxcbiAgICAgIHBhcnRpdGlvbktleTogeyBuYW1lOiBcInVybF9oYXNoXCIsIHR5cGU6IGR5bmFtb2RiLkF0dHJpYnV0ZVR5cGUuU1RSSU5HIH0sXG4gICAgICBiaWxsaW5nTW9kZTogZHluYW1vZGIuQmlsbGluZ01vZGUuUEFZX1BFUl9SRVFVRVNULFxuICAgICAgdGltZVRvTGl2ZUF0dHJpYnV0ZTogXCJleHBpcmVzX2F0XCIsXG4gICAgICByZW1vdmFsUG9saWN5OiBSZW1vdmFsUG9saWN5LlJFVEFJTlxuICAgIH0pO1xuICAgIHRoaXMuYWRkU2VydmljZVRhZ3MoY3Jhd2xVcmxzVGFibGUsIFwibWV0YWRhdGFcIiwgXCJjcmF3bC11cmwtZGVkdXBlXCIpO1xuXG4gICAgY3Jhd2xVcmxzVGFibGUuYWRkR2xvYmFsU2Vjb25kYXJ5SW5kZXgoe1xuICAgICAgaW5kZXhOYW1lOiBcImJ5LXN0YXR1c1wiLFxuICAgICAgcGFydGl0aW9uS2V5OiB7IG5hbWU6IFwic3RhdHVzXCIsIHR5cGU6IGR5bmFtb2RiLkF0dHJpYnV0ZVR5cGUuU1RSSU5HIH0sXG4gICAgICBzb3J0S2V5OiB7IG5hbWU6IFwidXBkYXRlZF9hdFwiLCB0eXBlOiBkeW5hbW9kYi5BdHRyaWJ1dGVUeXBlLlNUUklORyB9LFxuICAgICAgcHJvamVjdGlvblR5cGU6IGR5bmFtb2RiLlByb2plY3Rpb25UeXBlLkFMTFxuICAgIH0pO1xuXG4gICAgY29uc3QgZGVhZExldHRlclF1ZXVlID0gbmV3IHNxcy5RdWV1ZSh0aGlzLCBcIkNyYXdsRGVhZExldHRlclF1ZXVlXCIsIHtcbiAgICAgIHF1ZXVlTmFtZTogbmFtZShcImNyYXdsLWRscVwiKSxcbiAgICAgIHJldGVudGlvblBlcmlvZDogRHVyYXRpb24uZGF5cygxNClcbiAgICB9KTtcbiAgICB0aGlzLmFkZFNlcnZpY2VUYWdzKGRlYWRMZXR0ZXJRdWV1ZSwgXCJxdWV1ZVwiLCBcImNyYXdsLWRlYWQtbGV0dGVyXCIpO1xuXG4gICAgY29uc3QgY3Jhd2xRdWV1ZSA9IG5ldyBzcXMuUXVldWUodGhpcywgXCJDcmF3bFF1ZXVlXCIsIHtcbiAgICAgIHF1ZXVlTmFtZTogbmFtZShcImNyYXdsLWZyb250aWVyXCIpLFxuICAgICAgdmlzaWJpbGl0eVRpbWVvdXQ6IER1cmF0aW9uLm1pbnV0ZXMoNSksXG4gICAgICByZXRlbnRpb25QZXJpb2Q6IER1cmF0aW9uLmRheXMoNCksXG4gICAgICBkZWFkTGV0dGVyUXVldWU6IHtcbiAgICAgICAgcXVldWU6IGRlYWRMZXR0ZXJRdWV1ZSxcbiAgICAgICAgbWF4UmVjZWl2ZUNvdW50OiAzXG4gICAgICB9XG4gICAgfSk7XG4gICAgdGhpcy5hZGRTZXJ2aWNlVGFncyhjcmF3bFF1ZXVlLCBcInF1ZXVlXCIsIFwiY3Jhd2wtZnJvbnRpZXJcIik7XG5cbiAgICBjb25zdCBjcmF3bGVyUmVwb3NpdG9yeSA9IHRoaXMucmVwb3NpdG9yeShcIkNyYXdsZXJSZXBvc2l0b3J5XCIsIG5hbWUoXCJjcmF3bGVyXCIpKTtcbiAgICBjb25zdCBpbmRleGVyUmVwb3NpdG9yeSA9IHRoaXMucmVwb3NpdG9yeShcIkluZGV4ZXJSZXBvc2l0b3J5XCIsIG5hbWUoXCJpbmRleGVyXCIpKTtcbiAgICBjb25zdCBzZWFyY2hSZXBvc2l0b3J5ID0gdGhpcy5yZXBvc2l0b3J5KFwiU2VhcmNoQXBpUmVwb3NpdG9yeVwiLCBuYW1lKFwic2VhcmNoLWFwaVwiKSk7XG4gICAgdGhpcy5hZGRTZXJ2aWNlVGFncyhjcmF3bGVyUmVwb3NpdG9yeSwgXCJjcmF3bGVyXCIsIFwiY29udGFpbmVyLWltYWdlXCIpO1xuICAgIHRoaXMuYWRkU2VydmljZVRhZ3MoaW5kZXhlclJlcG9zaXRvcnksIFwiaW5kZXhlclwiLCBcImNvbnRhaW5lci1pbWFnZVwiKTtcbiAgICB0aGlzLmFkZFNlcnZpY2VUYWdzKHNlYXJjaFJlcG9zaXRvcnksIFwic2VhcmNoLWFwaVwiLCBcImNvbnRhaW5lci1pbWFnZVwiKTtcblxuICAgIGNvbnN0IHZwYyA9IG5ldyBlYzIuVnBjKHRoaXMsIFwiVnBjXCIsIHtcbiAgICAgIHZwY05hbWU6IG5hbWUoXCJ2cGNcIiksXG4gICAgICBuYXRHYXRld2F5czogMCxcbiAgICAgIG1heEF6czogMixcbiAgICAgIHN1Ym5ldENvbmZpZ3VyYXRpb246IFtcbiAgICAgICAge1xuICAgICAgICAgIG5hbWU6IFwicHVibGljXCIsXG4gICAgICAgICAgc3VibmV0VHlwZTogZWMyLlN1Ym5ldFR5cGUuUFVCTElDXG4gICAgICAgIH1cbiAgICAgIF1cbiAgICB9KTtcbiAgICB0aGlzLmFkZFNlcnZpY2VUYWdzKHZwYywgXCJuZXR3b3JrXCIsIFwicHVibGljLXZwY1wiKTtcblxuICAgIGNvbnN0IGNsdXN0ZXIgPSBuZXcgZWNzLkNsdXN0ZXIodGhpcywgXCJDbHVzdGVyXCIsIHtcbiAgICAgIGNsdXN0ZXJOYW1lOiBuYW1lKFwiY2x1c3RlclwiKSxcbiAgICAgIHZwYyxcbiAgICAgIGNvbnRhaW5lckluc2lnaHRzVjI6IGVjcy5Db250YWluZXJJbnNpZ2h0cy5FTkFCTEVEXG4gICAgfSk7XG4gICAgdGhpcy5hZGRTZXJ2aWNlVGFncyhjbHVzdGVyLCBcImNvbXB1dGVcIiwgXCJlY3MtY2x1c3RlclwiKTtcblxuICAgIGNvbnN0IGxvZ0dyb3VwID0gbmV3IGxvZ3MuTG9nR3JvdXAodGhpcywgXCJTZXJ2aWNlTG9nc1wiLCB7XG4gICAgICBsb2dHcm91cE5hbWU6IGAvYXJ4aXZpc3QvJHtwcm9wcy5wcm9qZWN0TmFtZX1gLFxuICAgICAgcmV0ZW50aW9uOiBsb2dzLlJldGVudGlvbkRheXMuT05FX1dFRUssXG4gICAgICByZW1vdmFsUG9saWN5OiBSZW1vdmFsUG9saWN5LkRFU1RST1lcbiAgICB9KTtcbiAgICB0aGlzLmFkZFNlcnZpY2VUYWdzKGxvZ0dyb3VwLCBcIm9ic2VydmFiaWxpdHlcIiwgXCJzZXJ2aWNlLWxvZ3NcIik7XG5cbiAgICBjb25zdCBjcmF3bGVyVGFzayA9IHRoaXMud29ya2VyVGFzayhcIkNyYXdsZXJUYXNrXCIsIHtcbiAgICAgIGZhbWlseTogbmFtZShcImNyYXdsZXJcIiksXG4gICAgICByZXBvc2l0b3J5OiBjcmF3bGVyUmVwb3NpdG9yeSxcbiAgICAgIGNvbW1hbmQ6IFtcbiAgICAgICAgXCItLXN0b3JhZ2VcIixcbiAgICAgICAgXCJhd3NcIixcbiAgICAgICAgXCItLWNyYXdsLWlkXCIsXG4gICAgICAgIGNyYXdsSWQsXG4gICAgICAgIFwiLS1tYXgtcGFnZXNcIixcbiAgICAgICAgU3RyaW5nKGNyYXdsTWF4UGFnZXMpLFxuICAgICAgICBcIi0tbWF4LWRlcHRoXCIsXG4gICAgICAgIFN0cmluZyhjcmF3bE1heERlcHRoKSxcbiAgICAgICAgXCItLWRlbGF5LW1zXCIsXG4gICAgICAgIFN0cmluZyhjcmF3bERlbGF5TXMpXG4gICAgICBdLFxuICAgICAgbG9nR3JvdXAsXG4gICAgICBlbnZpcm9ubWVudDoge1xuICAgICAgICBBUlhJVklTVF9TVE9SQUdFX01PREU6IFwiYXdzXCIsXG4gICAgICAgIEFSWElWSVNUX0NSQVdMX0lEOiBjcmF3bElkLFxuICAgICAgICBBUlhJVklTVF9FTVBUWV9SRUNFSVZFX0xJTUlUOiBTdHJpbmcoY3Jhd2xFbXB0eVJlY2VpdmVMaW1pdCksXG4gICAgICAgIEFSWElWSVNUX0RBVEFfQlVDS0VUOiBkYXRhQnVja2V0LmJ1Y2tldE5hbWUsXG4gICAgICAgIEFSWElWSVNUX1BBR0VTX1RBQkxFOiBwYWdlc1RhYmxlLnRhYmxlTmFtZSxcbiAgICAgICAgQVJYSVZJU1RfQ1JBV0xfVVJMU19UQUJMRTogY3Jhd2xVcmxzVGFibGUudGFibGVOYW1lLFxuICAgICAgICBBUlhJVklTVF9DUkFXTF9RVUVVRV9VUkw6IGNyYXdsUXVldWUucXVldWVVcmxcbiAgICAgIH1cbiAgICB9KTtcbiAgICB0aGlzLmFkZFNlcnZpY2VUYWdzKGNyYXdsZXJUYXNrLCBcImNyYXdsZXJcIiwgXCJ3b3JrZXItdGFza1wiKTtcblxuICAgIGNvbnN0IGluZGV4ZXJUYXNrID0gdGhpcy53b3JrZXJUYXNrKFwiSW5kZXhlclRhc2tcIiwge1xuICAgICAgZmFtaWx5OiBuYW1lKFwiaW5kZXhlclwiKSxcbiAgICAgIHJlcG9zaXRvcnk6IGluZGV4ZXJSZXBvc2l0b3J5LFxuICAgICAgY29tbWFuZDogW1wiLS1zdG9yYWdlXCIsIFwiYXdzXCJdLFxuICAgICAgY3B1OiAxMDI0LFxuICAgICAgbWVtb3J5TGltaXRNaUI6IDgxOTIsXG4gICAgICBsb2dHcm91cCxcbiAgICAgIGVudmlyb25tZW50OiB7XG4gICAgICAgIEFSWElWSVNUX1NUT1JBR0VfTU9ERTogXCJhd3NcIixcbiAgICAgICAgQVJYSVZJU1RfREFUQV9CVUNLRVQ6IGRhdGFCdWNrZXQuYnVja2V0TmFtZSxcbiAgICAgICAgQVJYSVZJU1RfUEFHRVNfVEFCTEU6IHBhZ2VzVGFibGUudGFibGVOYW1lLFxuICAgICAgICBBUlhJVklTVF9DUkFXTF9VUkxTX1RBQkxFOiBjcmF3bFVybHNUYWJsZS50YWJsZU5hbWUsXG4gICAgICAgIEFSWElWSVNUX0FDVElWRV9JTkRFWF9LRVk6IFwiaW5kZXhlcy9hY3RpdmUvbWFuaWZlc3QuanNvblwiXG4gICAgICB9XG4gICAgfSk7XG4gICAgdGhpcy5hZGRTZXJ2aWNlVGFncyhpbmRleGVyVGFzaywgXCJpbmRleGVyXCIsIFwid29ya2VyLXRhc2tcIik7XG5cbiAgICBkYXRhQnVja2V0LmdyYW50UmVhZFdyaXRlKGNyYXdsZXJUYXNrLnRhc2tSb2xlKTtcbiAgICBkYXRhQnVja2V0LmdyYW50UmVhZFdyaXRlKGluZGV4ZXJUYXNrLnRhc2tSb2xlKTtcbiAgICBwYWdlc1RhYmxlLmdyYW50UmVhZFdyaXRlRGF0YShjcmF3bGVyVGFzay50YXNrUm9sZSk7XG4gICAgcGFnZXNUYWJsZS5ncmFudFJlYWRXcml0ZURhdGEoaW5kZXhlclRhc2sudGFza1JvbGUpO1xuICAgIGNyYXdsVXJsc1RhYmxlLmdyYW50UmVhZFdyaXRlRGF0YShjcmF3bGVyVGFzay50YXNrUm9sZSk7XG4gICAgY3Jhd2xVcmxzVGFibGUuZ3JhbnRSZWFkV3JpdGVEYXRhKGluZGV4ZXJUYXNrLnRhc2tSb2xlKTtcbiAgICBjcmF3bFF1ZXVlLmdyYW50Q29uc3VtZU1lc3NhZ2VzKGNyYXdsZXJUYXNrLnRhc2tSb2xlKTtcbiAgICBjcmF3bFF1ZXVlLmdyYW50U2VuZE1lc3NhZ2VzKGNyYXdsZXJUYXNrLnRhc2tSb2xlKTtcblxuICAgIGNvbnN0IHNlYXJjaFRhc2sgPSBuZXcgZWNzLkZhcmdhdGVUYXNrRGVmaW5pdGlvbih0aGlzLCBcIlNlYXJjaFRhc2tcIiwge1xuICAgICAgZmFtaWx5OiBuYW1lKFwic2VhcmNoLWFwaVwiKSxcbiAgICAgIGNwdTogMTAyNCxcbiAgICAgIG1lbW9yeUxpbWl0TWlCOiA0MDk2XG4gICAgfSk7XG4gICAgdGhpcy5hZGRTZXJ2aWNlVGFncyhzZWFyY2hUYXNrLCBcInNlYXJjaC1hcGlcIiwgXCJhcGktdGFza1wiKTtcblxuICAgIHNlYXJjaFRhc2suYWRkQ29udGFpbmVyKFwiU2VhcmNoQXBpXCIsIHtcbiAgICAgIGltYWdlOiBlY3MuQ29udGFpbmVySW1hZ2UuZnJvbUVjclJlcG9zaXRvcnkoc2VhcmNoUmVwb3NpdG9yeSwgXCJsYXRlc3RcIiksXG4gICAgICBsb2dnaW5nOiBlY3MuTG9nRHJpdmVycy5hd3NMb2dzKHtcbiAgICAgICAgc3RyZWFtUHJlZml4OiBcInNlYXJjaC1hcGlcIixcbiAgICAgICAgbG9nR3JvdXBcbiAgICAgIH0pLFxuICAgICAgZW52aXJvbm1lbnQ6IHtcbiAgICAgICAgQVJYSVZJU1RfU1RPUkFHRV9NT0RFOiBcImF3c1wiLFxuICAgICAgICBBUlhJVklTVF9EQVRBX0JVQ0tFVDogZGF0YUJ1Y2tldC5idWNrZXROYW1lLFxuICAgICAgICBBUlhJVklTVF9BQ1RJVkVfSU5ERVhfS0VZOiBcImluZGV4ZXMvYWN0aXZlL21hbmlmZXN0Lmpzb25cIixcbiAgICAgICAgQVJYSVZJU1RfQ09SU19PUklHSU46IGRlbW9Db3JzT3JpZ2luXG4gICAgICB9LFxuICAgICAgY29tbWFuZDogW1wiLS1zdG9yYWdlXCIsIFwiYXdzXCIsIFwiLS1iaW5kXCIsIFwiMC4wLjAuMDozMDAwXCJdLFxuICAgICAgcG9ydE1hcHBpbmdzOiBbeyBjb250YWluZXJQb3J0OiAzMDAwIH1dXG4gICAgfSk7XG5cbiAgICBkYXRhQnVja2V0LmdyYW50UmVhZChzZWFyY2hUYXNrLnRhc2tSb2xlKTtcblxuICAgIGNvbnN0IHNlYXJjaExvYWRCYWxhbmNlciA9IG5ldyBlbGJ2Mi5BcHBsaWNhdGlvbkxvYWRCYWxhbmNlcih0aGlzLCBcIlNlYXJjaExvYWRCYWxhbmNlclwiLCB7XG4gICAgICBsb2FkQmFsYW5jZXJOYW1lOiBuYW1lKFwic2VhcmNoLWFwaVwiKSxcbiAgICAgIHZwYyxcbiAgICAgIGludGVybmV0RmFjaW5nOiB0cnVlXG4gICAgfSk7XG4gICAgdGhpcy5hZGRTZXJ2aWNlVGFncyhzZWFyY2hMb2FkQmFsYW5jZXIsIFwic2VhcmNoLWFwaVwiLCBcInB1YmxpYy1sb2FkLWJhbGFuY2VyXCIpO1xuXG4gICAgY29uc3Qgc2VhcmNoTGlzdGVuZXIgPSBzZWFyY2hMb2FkQmFsYW5jZXIuYWRkTGlzdGVuZXIoXCJTZWFyY2hIdHRwTGlzdGVuZXJcIiwge1xuICAgICAgcG9ydDogODAsXG4gICAgICBvcGVuOiB0cnVlXG4gICAgfSk7XG4gICAgdGhpcy5hZGRTZXJ2aWNlVGFncyhzZWFyY2hMaXN0ZW5lciwgXCJzZWFyY2gtYXBpXCIsIFwiaHR0cC1saXN0ZW5lclwiKTtcblxuICAgIC8vIEtlZXAgdGhlIHB1YmxpYyBlbmRwb2ludCBpbiBwbGFjZSB3aGlsZSBhbGxvd2luZyBkZW1vIGVudmlyb25tZW50cyB0byBpZGxlIGF0IHplcm8gdGFza3MuXG4gICAgY29uc3Qgc2VhcmNoU2VydmljZSA9IG5ldyBlY3MuRmFyZ2F0ZVNlcnZpY2UodGhpcywgXCJTZWFyY2hTZXJ2aWNlXCIsIHtcbiAgICAgIHNlcnZpY2VOYW1lOiBuYW1lKFwic2VhcmNoLWFwaVwiKSxcbiAgICAgIGNsdXN0ZXIsXG4gICAgICB0YXNrRGVmaW5pdGlvbjogc2VhcmNoVGFzayxcbiAgICAgIGRlc2lyZWRDb3VudDogc2VhcmNoRGVzaXJlZENvdW50LFxuICAgICAgY2lyY3VpdEJyZWFrZXI6IHsgcm9sbGJhY2s6IHRydWUgfSxcbiAgICAgIG1pbkhlYWx0aHlQZXJjZW50OiAxMDAsXG4gICAgICBhc3NpZ25QdWJsaWNJcDogdHJ1ZSxcbiAgICAgIHZwY1N1Ym5ldHM6IHsgc3VibmV0VHlwZTogZWMyLlN1Ym5ldFR5cGUuUFVCTElDIH1cbiAgICB9KTtcbiAgICB0aGlzLmFkZFNlcnZpY2VUYWdzKHNlYXJjaFNlcnZpY2UsIFwic2VhcmNoLWFwaVwiLCBcImFwaS1zZXJ2aWNlXCIpO1xuXG4gICAgY29uc3Qgc2VhcmNoVGFyZ2V0R3JvdXAgPSBzZWFyY2hMaXN0ZW5lci5hZGRUYXJnZXRzKFwiU2VhcmNoVGFyZ2V0c1wiLCB7XG4gICAgICBwb3J0OiAzMDAwLFxuICAgICAgcHJvdG9jb2w6IGVsYnYyLkFwcGxpY2F0aW9uUHJvdG9jb2wuSFRUUCxcbiAgICAgIHRhcmdldHM6IFtzZWFyY2hTZXJ2aWNlXVxuICAgIH0pO1xuICAgIHRoaXMuYWRkU2VydmljZVRhZ3Moc2VhcmNoVGFyZ2V0R3JvdXAsIFwic2VhcmNoLWFwaVwiLCBcImFwaS10YXJnZXQtZ3JvdXBcIik7XG5cbiAgICBzZWFyY2hUYXJnZXRHcm91cC5jb25maWd1cmVIZWFsdGhDaGVjayh7XG4gICAgICBwYXRoOiBcIi9oZWFsdGhcIixcbiAgICAgIGhlYWx0aHlIdHRwQ29kZXM6IFwiMjAwXCIsXG4gICAgICBpbnRlcnZhbDogRHVyYXRpb24uc2Vjb25kcygzMClcbiAgICB9KTtcblxuICAgIGNvbnN0IG9wZW5BaUFwaUtleVNlY3JldCA9IHNlY3JldHNtYW5hZ2VyLlNlY3JldC5mcm9tU2VjcmV0TmFtZVYyKFxuICAgICAgdGhpcyxcbiAgICAgIFwiT3BlbkFpQXBpS2V5U2VjcmV0XCIsXG4gICAgICBvcGVuQWlBcGlLZXlTZWNyZXROYW1lXG4gICAgKTtcblxuICAgIC8vIFRoZSBhZ2VudCByZW1haW5zIGEgc2VwYXJhdGUgYm91bmRhcnkgc28gdGhlIGxlYXJuaW5nIGdyYXBoIGNhbiBldm9sdmVcbiAgICAvLyB3aXRob3V0IGNvdXBsaW5nIExMTSBiZWhhdmlvciB0byB0aGUgdHJhZGl0aW9uYWwgUnVzdCBzZWFyY2ggQVBJLlxuICAgIGNvbnN0IGFnZW50RnVuY3Rpb24gPSBuZXcgbGFtYmRhLkRvY2tlckltYWdlRnVuY3Rpb24odGhpcywgXCJBZ2VudEZ1bmN0aW9uXCIsIHtcbiAgICAgIGZ1bmN0aW9uTmFtZTogbmFtZShcImFnZW50LWFwaVwiKSxcbiAgICAgIGNvZGU6IGxhbWJkYS5Eb2NrZXJJbWFnZUNvZGUuZnJvbUltYWdlQXNzZXQocGF0aC5qb2luKF9fZGlybmFtZSwgXCIuLi8uLi9hcnhpdmlzdC1hZ2VudFwiKSksXG4gICAgICBtZW1vcnlTaXplOiAxMDI0LFxuICAgICAgdGltZW91dDogRHVyYXRpb24uc2Vjb25kcyhhZ2VudFRpbWVvdXRTZWNvbmRzKSxcbiAgICAgIGVudmlyb25tZW50OiB7XG4gICAgICAgIEFSWElWSVNUX1NUT1JBR0VfTU9ERTogXCJhd3NcIixcbiAgICAgICAgQVJYSVZJU1RfREFUQV9CVUNLRVQ6IGRhdGFCdWNrZXQuYnVja2V0TmFtZSxcbiAgICAgICAgQVJYSVZJU1RfUEFHRVNfVEFCTEU6IHBhZ2VzVGFibGUudGFibGVOYW1lLFxuICAgICAgICBBUlhJVklTVF9TRUFSQ0hfQVBJX0JBU0VfVVJMOiBgaHR0cDovLyR7c2VhcmNoTG9hZEJhbGFuY2VyLmxvYWRCYWxhbmNlckRuc05hbWV9YCxcbiAgICAgICAgT1BFTkFJX0FQSV9LRVlfU0VDUkVUX05BTUU6IG9wZW5BaUFwaUtleVNlY3JldE5hbWVcbiAgICAgIH1cbiAgICB9KTtcbiAgICB0aGlzLmFkZFNlcnZpY2VUYWdzKGFnZW50RnVuY3Rpb24sIFwiYWdlbnQtYXBpXCIsIFwibGFtYmRhLWZ1bmN0aW9uXCIpO1xuXG4gICAgb3BlbkFpQXBpS2V5U2VjcmV0LmdyYW50UmVhZChhZ2VudEZ1bmN0aW9uKTtcbiAgICBkYXRhQnVja2V0LmdyYW50UmVhZChhZ2VudEZ1bmN0aW9uKTtcbiAgICBwYWdlc1RhYmxlLmdyYW50UmVhZERhdGEoYWdlbnRGdW5jdGlvbik7XG5cbiAgICBjb25zdCBhZ2VudEFwaSA9IG5ldyBhcGlnYXRld2F5djIuSHR0cEFwaSh0aGlzLCBcIkFnZW50SHR0cEFwaVwiLCB7XG4gICAgICBhcGlOYW1lOiBuYW1lKFwiYWdlbnQtYXBpXCIpLFxuICAgICAgY29yc1ByZWZsaWdodDoge1xuICAgICAgICBhbGxvd0hlYWRlcnM6IFtcImNvbnRlbnQtdHlwZVwiXSxcbiAgICAgICAgYWxsb3dNZXRob2RzOiBbXG4gICAgICAgICAgYXBpZ2F0ZXdheXYyLkNvcnNIdHRwTWV0aG9kLkdFVCxcbiAgICAgICAgICBhcGlnYXRld2F5djIuQ29yc0h0dHBNZXRob2QuUE9TVCxcbiAgICAgICAgICBhcGlnYXRld2F5djIuQ29yc0h0dHBNZXRob2QuT1BUSU9OU1xuICAgICAgICBdLFxuICAgICAgICBhbGxvd09yaWdpbnM6IFtkZW1vQ29yc09yaWdpbl1cbiAgICAgIH1cbiAgICB9KTtcbiAgICB0aGlzLmFkZFNlcnZpY2VUYWdzKGFnZW50QXBpLCBcImFnZW50LWFwaVwiLCBcImh0dHAtYXBpXCIpO1xuXG4gICAgY29uc3QgYWdlbnRJbnRlZ3JhdGlvbiA9IG5ldyBpbnRlZ3JhdGlvbnMuSHR0cExhbWJkYUludGVncmF0aW9uKFxuICAgICAgXCJBZ2VudExhbWJkYUludGVncmF0aW9uXCIsXG4gICAgICBhZ2VudEZ1bmN0aW9uXG4gICAgKTtcblxuICAgIGFnZW50QXBpLmFkZFJvdXRlcyh7XG4gICAgICBwYXRoOiBcIi9hZ2VudC9oZWFsdGhcIixcbiAgICAgIG1ldGhvZHM6IFthcGlnYXRld2F5djIuSHR0cE1ldGhvZC5HRVRdLFxuICAgICAgaW50ZWdyYXRpb246IGFnZW50SW50ZWdyYXRpb25cbiAgICB9KTtcbiAgICBhZ2VudEFwaS5hZGRSb3V0ZXMoe1xuICAgICAgcGF0aDogXCIvYWdlbnQvc2VhcmNoXCIsXG4gICAgICBtZXRob2RzOiBbYXBpZ2F0ZXdheXYyLkh0dHBNZXRob2QuUE9TVF0sXG4gICAgICBpbnRlZ3JhdGlvbjogYWdlbnRJbnRlZ3JhdGlvblxuICAgIH0pO1xuXG4gICAgY29uc3QgY3Jhd2xEbHFBbGFybSA9IG5ldyBjbG91ZHdhdGNoLkFsYXJtKHRoaXMsIFwiQ3Jhd2xEbHFBbGFybVwiLCB7XG4gICAgICBhbGFybU5hbWU6IG5hbWUoXCJjcmF3bC1kbHEtdmlzaWJsZVwiKSxcbiAgICAgIG1ldHJpYzogZGVhZExldHRlclF1ZXVlLm1ldHJpY0FwcHJveGltYXRlTnVtYmVyT2ZNZXNzYWdlc1Zpc2libGUoKSxcbiAgICAgIHRocmVzaG9sZDogMSxcbiAgICAgIGV2YWx1YXRpb25QZXJpb2RzOiAxXG4gICAgfSk7XG4gICAgdGhpcy5hZGRTZXJ2aWNlVGFncyhjcmF3bERscUFsYXJtLCBcIm9ic2VydmFiaWxpdHlcIiwgXCJjcmF3bC1kbHEtYWxhcm1cIik7XG5cbiAgICBpZiAoYnVkZ2V0RW1haWwgJiYgYnVkZ2V0RW1haWwudHJpbSgpICE9PSBcIlwiKSB7XG4gICAgICBjb25zdCBtb250aGx5QnVkZ2V0ID0gbmV3IGJ1ZGdldHMuQ2ZuQnVkZ2V0KHRoaXMsIFwiTW9udGhseUJ1ZGdldFwiLCB7XG4gICAgICAgIGJ1ZGdldDoge1xuICAgICAgICAgIGJ1ZGdldE5hbWU6IG5hbWUoXCJtb250aGx5LWRlbW8tYnVkZ2V0XCIpLFxuICAgICAgICAgIGJ1ZGdldExpbWl0OiB7XG4gICAgICAgICAgICBhbW91bnQ6IG1vbnRobHlCdWRnZXRVc2QsXG4gICAgICAgICAgICB1bml0OiBcIlVTRFwiXG4gICAgICAgICAgfSxcbiAgICAgICAgICB0aW1lVW5pdDogXCJNT05USExZXCIsXG4gICAgICAgICAgYnVkZ2V0VHlwZTogXCJDT1NUXCJcbiAgICAgICAgfSxcbiAgICAgICAgbm90aWZpY2F0aW9uc1dpdGhTdWJzY3JpYmVyczogW1xuICAgICAgICAgIHtcbiAgICAgICAgICAgIG5vdGlmaWNhdGlvbjoge1xuICAgICAgICAgICAgICBub3RpZmljYXRpb25UeXBlOiBcIkFDVFVBTFwiLFxuICAgICAgICAgICAgICBjb21wYXJpc29uT3BlcmF0b3I6IFwiR1JFQVRFUl9USEFOXCIsXG4gICAgICAgICAgICAgIHRocmVzaG9sZDogODAsXG4gICAgICAgICAgICAgIHRocmVzaG9sZFR5cGU6IFwiUEVSQ0VOVEFHRVwiXG4gICAgICAgICAgICB9LFxuICAgICAgICAgICAgc3Vic2NyaWJlcnM6IFtcbiAgICAgICAgICAgICAge1xuICAgICAgICAgICAgICAgIHN1YnNjcmlwdGlvblR5cGU6IFwiRU1BSUxcIixcbiAgICAgICAgICAgICAgICBhZGRyZXNzOiBidWRnZXRFbWFpbFxuICAgICAgICAgICAgICB9XG4gICAgICAgICAgICBdXG4gICAgICAgICAgfVxuICAgICAgICBdXG4gICAgICB9KTtcbiAgICAgIHRoaXMuYWRkU2VydmljZVRhZ3MobW9udGhseUJ1ZGdldCwgXCJvYnNlcnZhYmlsaXR5XCIsIFwibW9udGhseS1idWRnZXRcIik7XG4gICAgfVxuXG4gICAgbmV3IGNkay5DZm5PdXRwdXQodGhpcywgXCJEYXRhQnVja2V0TmFtZVwiLCB7IHZhbHVlOiBkYXRhQnVja2V0LmJ1Y2tldE5hbWUgfSk7XG4gICAgbmV3IGNkay5DZm5PdXRwdXQodGhpcywgXCJQYWdlc1RhYmxlTmFtZVwiLCB7IHZhbHVlOiBwYWdlc1RhYmxlLnRhYmxlTmFtZSB9KTtcbiAgICBuZXcgY2RrLkNmbk91dHB1dCh0aGlzLCBcIkNyYXdsVXJsc1RhYmxlTmFtZVwiLCB7IHZhbHVlOiBjcmF3bFVybHNUYWJsZS50YWJsZU5hbWUgfSk7XG4gICAgbmV3IGNkay5DZm5PdXRwdXQodGhpcywgXCJDcmF3bFF1ZXVlVXJsXCIsIHsgdmFsdWU6IGNyYXdsUXVldWUucXVldWVVcmwgfSk7XG4gICAgbmV3IGNkay5DZm5PdXRwdXQodGhpcywgXCJTZWFyY2hBcGlVcmxcIiwge1xuICAgICAgdmFsdWU6IGBodHRwOi8vJHtzZWFyY2hMb2FkQmFsYW5jZXIubG9hZEJhbGFuY2VyRG5zTmFtZX1gXG4gICAgfSk7XG4gICAgbmV3IGNkay5DZm5PdXRwdXQodGhpcywgXCJBZ2VudEFwaVVybFwiLCB7IHZhbHVlOiBhZ2VudEFwaS5hcGlFbmRwb2ludCB9KTtcbiAgICBuZXcgY2RrLkNmbk91dHB1dCh0aGlzLCBcIk9wZW5BaUFwaUtleVNlY3JldE5hbWVcIiwgeyB2YWx1ZTogb3BlbkFpQXBpS2V5U2VjcmV0TmFtZSB9KTtcbiAgICBuZXcgY2RrLkNmbk91dHB1dCh0aGlzLCBcIkNyYXdsZXJJbWFnZUJ1aWxkXCIsIHtcbiAgICAgIHZhbHVlOiBgZG9ja2VyIGJ1aWxkIC0tYnVpbGQtYXJnIEJJTj1hcnhpdmlzdC1jcmF3bGVyIC10ICR7Y3Jhd2xlclJlcG9zaXRvcnkucmVwb3NpdG9yeVVyaX06bGF0ZXN0IC5gXG4gICAgfSk7XG4gICAgbmV3IGNkay5DZm5PdXRwdXQodGhpcywgXCJJbmRleGVySW1hZ2VCdWlsZFwiLCB7XG4gICAgICB2YWx1ZTogYGRvY2tlciBidWlsZCAtLWJ1aWxkLWFyZyBCSU49YXJ4aXZpc3QtaW5kZXhlciAtdCAke2luZGV4ZXJSZXBvc2l0b3J5LnJlcG9zaXRvcnlVcml9OmxhdGVzdCAuYFxuICAgIH0pO1xuICAgIG5ldyBjZGsuQ2ZuT3V0cHV0KHRoaXMsIFwiU2VhcmNoQXBpSW1hZ2VCdWlsZFwiLCB7XG4gICAgICB2YWx1ZTogYGRvY2tlciBidWlsZCAtLWJ1aWxkLWFyZyBCSU49YXJ4aXZpc3Qtc2VhcmNoLWFwaSAtdCAke3NlYXJjaFJlcG9zaXRvcnkucmVwb3NpdG9yeVVyaX06bGF0ZXN0IC5gXG4gICAgfSk7XG4gICAgbmV3IGNkay5DZm5PdXRwdXQodGhpcywgXCJDcmF3bGVyTWF4Q2FwYWNpdHlcIiwgeyB2YWx1ZTogU3RyaW5nKGNyYXdsZXJNYXhDYXBhY2l0eSkgfSk7XG4gICAgbmV3IGNkay5DZm5PdXRwdXQodGhpcywgXCJDcmF3bElkXCIsIHsgdmFsdWU6IGNyYXdsSWQgfSk7XG4gICAgbmV3IGNkay5DZm5PdXRwdXQodGhpcywgXCJDcmF3bE1heFBhZ2VzXCIsIHsgdmFsdWU6IFN0cmluZyhjcmF3bE1heFBhZ2VzKSB9KTtcbiAgfVxuXG4gIHByaXZhdGUgYWRkR2xvYmFsQ29zdFRhZ3MocHJvamVjdE5hbWU6IHN0cmluZyk6IHZvaWQge1xuICAgIGNkay5UYWdzLm9mKHRoaXMpLmFkZChcIlByb2plY3RcIiwgcHJvamVjdE5hbWUpO1xuICAgIGNkay5UYWdzLm9mKHRoaXMpLmFkZChcIkVudmlyb25tZW50XCIsIFwiZGVtb1wiKTtcbiAgICBjZGsuVGFncy5vZih0aGlzKS5hZGQoXCJNYW5hZ2VkQnlcIiwgXCJjZGtcIik7XG4gICAgY2RrLlRhZ3Mub2YodGhpcykuYWRkKFwiT3duZXJcIiwgXCJrZXZpdXNcIik7XG4gICAgY2RrLlRhZ3Mub2YodGhpcykuYWRkKFwiQ29zdENlbnRlclwiLCBcImFyeGl2aXN0LWRlbW9cIik7XG4gIH1cblxuICBwcml2YXRlIGFkZFNlcnZpY2VUYWdzKHJlc291cmNlOiBDb25zdHJ1Y3QsIHNlcnZpY2U6IHN0cmluZywgY29tcG9uZW50OiBzdHJpbmcpOiB2b2lkIHtcbiAgICBjZGsuVGFncy5vZihyZXNvdXJjZSkuYWRkKFwiU2VydmljZVwiLCBzZXJ2aWNlKTtcbiAgICBjZGsuVGFncy5vZihyZXNvdXJjZSkuYWRkKFwiQ29tcG9uZW50XCIsIGNvbXBvbmVudCk7XG4gIH1cblxuICBwcml2YXRlIHJlcG9zaXRvcnkoaWQ6IHN0cmluZywgcmVwb3NpdG9yeU5hbWU6IHN0cmluZyk6IGVjci5SZXBvc2l0b3J5IHtcbiAgICByZXR1cm4gbmV3IGVjci5SZXBvc2l0b3J5KHRoaXMsIGlkLCB7XG4gICAgICByZXBvc2l0b3J5TmFtZSxcbiAgICAgIGltYWdlU2Nhbk9uUHVzaDogdHJ1ZSxcbiAgICAgIHJlbW92YWxQb2xpY3k6IFJlbW92YWxQb2xpY3kuREVTVFJPWSxcbiAgICAgIGVtcHR5T25EZWxldGU6IHRydWUsXG4gICAgICBsaWZlY3ljbGVSdWxlczogW1xuICAgICAgICB7XG4gICAgICAgICAgbWF4SW1hZ2VDb3VudDogNSxcbiAgICAgICAgICBkZXNjcmlwdGlvbjogXCJLZWVwIG9ubHkgcmVjZW50IGRlbW8gaW1hZ2VzLlwiXG4gICAgICAgIH1cbiAgICAgIF1cbiAgICB9KTtcbiAgfVxuXG4gIHByaXZhdGUgd29ya2VyVGFzayhcbiAgICBpZDogc3RyaW5nLFxuICAgIHByb3BzOiB7XG4gICAgICBmYW1pbHk6IHN0cmluZztcbiAgICAgIHJlcG9zaXRvcnk6IGVjci5JUmVwb3NpdG9yeTtcbiAgICAgIGNvbW1hbmQ6IHN0cmluZ1tdO1xuICAgICAgY3B1PzogbnVtYmVyO1xuICAgICAgbWVtb3J5TGltaXRNaUI/OiBudW1iZXI7XG4gICAgICBsb2dHcm91cDogbG9ncy5JTG9nR3JvdXA7XG4gICAgICBlbnZpcm9ubWVudDogUmVjb3JkPHN0cmluZywgc3RyaW5nPjtcbiAgICB9XG4gICk6IGVjcy5GYXJnYXRlVGFza0RlZmluaXRpb24ge1xuICAgIGNvbnN0IHRhc2sgPSBuZXcgZWNzLkZhcmdhdGVUYXNrRGVmaW5pdGlvbih0aGlzLCBpZCwge1xuICAgICAgZmFtaWx5OiBwcm9wcy5mYW1pbHksXG4gICAgICBjcHU6IHByb3BzLmNwdSA/PyA1MTIsXG4gICAgICBtZW1vcnlMaW1pdE1pQjogcHJvcHMubWVtb3J5TGltaXRNaUIgPz8gMTAyNFxuICAgIH0pO1xuXG4gICAgdGFzay5hZGRDb250YWluZXIoXCJXb3JrZXJcIiwge1xuICAgICAgaW1hZ2U6IGVjcy5Db250YWluZXJJbWFnZS5mcm9tRWNyUmVwb3NpdG9yeShwcm9wcy5yZXBvc2l0b3J5LCBcImxhdGVzdFwiKSxcbiAgICAgIGNvbW1hbmQ6IHByb3BzLmNvbW1hbmQubGVuZ3RoID4gMCA/IHByb3BzLmNvbW1hbmQgOiB1bmRlZmluZWQsXG4gICAgICBsb2dnaW5nOiBlY3MuTG9nRHJpdmVycy5hd3NMb2dzKHtcbiAgICAgICAgc3RyZWFtUHJlZml4OiBwcm9wcy5mYW1pbHksXG4gICAgICAgIGxvZ0dyb3VwOiBwcm9wcy5sb2dHcm91cFxuICAgICAgfSksXG4gICAgICBlbnZpcm9ubWVudDogcHJvcHMuZW52aXJvbm1lbnRcbiAgICB9KTtcblxuICAgIHJldHVybiB0YXNrO1xuICB9XG59XG4iXX0=