import * as cdk from "aws-cdk-lib";
import { Duration, RemovalPolicy, Stack, StackProps } from "aws-cdk-lib";
import * as budgets from "aws-cdk-lib/aws-budgets";
import * as cloudwatch from "aws-cdk-lib/aws-cloudwatch";
import * as dynamodb from "aws-cdk-lib/aws-dynamodb";
import * as ecr from "aws-cdk-lib/aws-ecr";
import * as ec2 from "aws-cdk-lib/aws-ec2";
import * as ecs from "aws-cdk-lib/aws-ecs";
import * as elbv2 from "aws-cdk-lib/aws-elasticloadbalancingv2";
import * as apigatewayv2 from "aws-cdk-lib/aws-apigatewayv2";
import * as integrations from "aws-cdk-lib/aws-apigatewayv2-integrations";
import * as lambda from "aws-cdk-lib/aws-lambda";
import * as logs from "aws-cdk-lib/aws-logs";
import * as s3 from "aws-cdk-lib/aws-s3";
import * as secretsmanager from "aws-cdk-lib/aws-secretsmanager";
import * as sqs from "aws-cdk-lib/aws-sqs";
import { Construct } from "constructs";
import * as path from "path";

interface ArxivistDemoStackProps extends StackProps {
  projectName: string;
}

export class ArxivistDemoStack extends Stack {
  constructor(scope: Construct, id: string, props: ArxivistDemoStackProps) {
    super(scope, id, props);

    const demoCorsOrigin = this.node.tryGetContext("demoCorsOrigin") ?? "*";
    const budgetEmail = this.node.tryGetContext("budgetEmail") as string | undefined;
    const monthlyBudgetUsd = Number(this.node.tryGetContext("monthlyBudgetUsd") ?? 90);
    const searchDesiredCount = Number(this.node.tryGetContext("searchDesiredCount") ?? 0);
    const crawlerMaxCapacity = Number(this.node.tryGetContext("crawlerMaxCapacity") ?? 4);
    const crawlId = String(this.node.tryGetContext("crawlId") ?? "demo-50k");
    const crawlMaxPages = Number(this.node.tryGetContext("crawlMaxPages") ?? 50_000);
    const crawlMaxDepth = Number(this.node.tryGetContext("crawlMaxDepth") ?? 8);
    const crawlDelayMs = Number(this.node.tryGetContext("crawlDelayMs") ?? 250);
    const crawlEmptyReceiveLimit = Number(this.node.tryGetContext("crawlEmptyReceiveLimit") ?? 30);
    const agentTimeoutSeconds = Number(this.node.tryGetContext("agentTimeoutSeconds") ?? 60);
    const openAiApiKeySecretName = String(
      this.node.tryGetContext("openAiApiKeySecretName") ?? `${props.projectName}/openai-api-key`
    );
    const name = (suffix: string) => `${props.projectName}-${suffix}`;

    this.addGlobalCostTags(props.projectName);

    // Corpus data is intentionally retained so compute can be destroyed without re-crawling.
    const dataBucket = new s3.Bucket(this, "DataBucket", {
      bucketName: `${props.projectName}-data-${this.account}-${this.region}`,
      blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
      encryption: s3.BucketEncryption.S3_MANAGED,
      enforceSSL: true,
      removalPolicy: RemovalPolicy.RETAIN,
      autoDeleteObjects: false,
      lifecycleRules: [
        {
          id: "expire-old-index-artifacts",
          prefix: "indexes/",
          noncurrentVersionExpiration: Duration.days(14)
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
      removalPolicy: RemovalPolicy.RETAIN
    });
    this.addServiceTags(pagesTable, "metadata", "pages-table");

    const crawlUrlsTable = new dynamodb.Table(this, "CrawlUrlsTable", {
      tableName: name("crawl-urls"),
      partitionKey: { name: "url_hash", type: dynamodb.AttributeType.STRING },
      billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
      timeToLiveAttribute: "expires_at",
      removalPolicy: RemovalPolicy.RETAIN
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
      retentionPeriod: Duration.days(14)
    });
    this.addServiceTags(deadLetterQueue, "queue", "crawl-dead-letter");

    const crawlQueue = new sqs.Queue(this, "CrawlQueue", {
      queueName: name("crawl-frontier"),
      visibilityTimeout: Duration.minutes(5),
      retentionPeriod: Duration.days(4),
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
      removalPolicy: RemovalPolicy.DESTROY
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
      interval: Duration.seconds(30)
    });

    const openAiApiKeySecret = secretsmanager.Secret.fromSecretNameV2(
      this,
      "OpenAiApiKeySecret",
      openAiApiKeySecretName
    );

    // The agent remains a separate boundary so the learning graph can evolve
    // without coupling LLM behavior to the traditional Rust search API.
    const agentFunction = new lambda.DockerImageFunction(this, "AgentFunction", {
      functionName: name("agent-api"),
      code: lambda.DockerImageCode.fromImageAsset(path.join(__dirname, "../../arxivist-agent")),
      memorySize: 1024,
      timeout: Duration.seconds(agentTimeoutSeconds),
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

    const agentIntegration = new integrations.HttpLambdaIntegration(
      "AgentLambdaIntegration",
      agentFunction
    );

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

  private addGlobalCostTags(projectName: string): void {
    cdk.Tags.of(this).add("Project", projectName);
    cdk.Tags.of(this).add("Environment", "demo");
    cdk.Tags.of(this).add("ManagedBy", "cdk");
    cdk.Tags.of(this).add("Owner", "kevius");
    cdk.Tags.of(this).add("CostCenter", "arxivist-demo");
  }

  private addServiceTags(resource: Construct, service: string, component: string): void {
    cdk.Tags.of(resource).add("Service", service);
    cdk.Tags.of(resource).add("Component", component);
  }

  private repository(id: string, repositoryName: string): ecr.Repository {
    return new ecr.Repository(this, id, {
      repositoryName,
      imageScanOnPush: true,
      removalPolicy: RemovalPolicy.DESTROY,
      emptyOnDelete: true,
      lifecycleRules: [
        {
          maxImageCount: 5,
          description: "Keep only recent demo images."
        }
      ]
    });
  }

  private workerTask(
    id: string,
    props: {
      family: string;
      repository: ecr.IRepository;
      command: string[];
      cpu?: number;
      memoryLimitMiB?: number;
      logGroup: logs.ILogGroup;
      environment: Record<string, string>;
    }
  ): ecs.FargateTaskDefinition {
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
