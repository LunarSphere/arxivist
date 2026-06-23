import * as cdk from "aws-cdk-lib";
import { Duration, RemovalPolicy, Stack, StackProps } from "aws-cdk-lib";
import * as budgets from "aws-cdk-lib/aws-budgets";
import * as cloudwatch from "aws-cdk-lib/aws-cloudwatch";
import * as dynamodb from "aws-cdk-lib/aws-dynamodb";
import * as ecr from "aws-cdk-lib/aws-ecr";
import * as ec2 from "aws-cdk-lib/aws-ec2";
import * as ecs from "aws-cdk-lib/aws-ecs";
import * as elbv2 from "aws-cdk-lib/aws-elasticloadbalancingv2";
import * as logs from "aws-cdk-lib/aws-logs";
import * as s3 from "aws-cdk-lib/aws-s3";
import * as sqs from "aws-cdk-lib/aws-sqs";
import { Construct } from "constructs";

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
    const name = (suffix: string) => `${props.projectName}-${suffix}`;

    const dataBucket = new s3.Bucket(this, "DataBucket", {
      bucketName: `${props.projectName}-data-${this.account}-${this.region}`,
      blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
      encryption: s3.BucketEncryption.S3_MANAGED,
      enforceSSL: true,
      removalPolicy: RemovalPolicy.DESTROY,
      autoDeleteObjects: true,
      lifecycleRules: [
        {
          id: "expire-demo-crawl-data",
          prefix: "crawl/",
          expiration: Duration.days(7)
        },
        {
          id: "expire-old-index-artifacts",
          prefix: "indexes/",
          noncurrentVersionExpiration: Duration.days(14)
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
      removalPolicy: RemovalPolicy.DESTROY
    });

    const crawlUrlsTable = new dynamodb.Table(this, "CrawlUrlsTable", {
      tableName: name("crawl-urls"),
      partitionKey: { name: "url_hash", type: dynamodb.AttributeType.STRING },
      billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
      timeToLiveAttribute: "expires_at",
      removalPolicy: RemovalPolicy.DESTROY
    });

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

    const crawlQueue = new sqs.Queue(this, "CrawlQueue", {
      queueName: name("crawl-frontier"),
      visibilityTimeout: Duration.minutes(5),
      retentionPeriod: Duration.days(4),
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
      removalPolicy: RemovalPolicy.DESTROY
    });

    const crawlerTask = this.workerTask("CrawlerTask", {
      family: name("crawler"),
      repository: crawlerRepository,
      command: [],
      logGroup,
      environment: {
        ARXIVIST_DATA_BUCKET: dataBucket.bucketName,
        ARXIVIST_PAGES_TABLE: pagesTable.tableName,
        ARXIVIST_CRAWL_URLS_TABLE: crawlUrlsTable.tableName,
        ARXIVIST_CRAWL_QUEUE_URL: crawlQueue.queueUrl
      }
    });

    const indexerTask = this.workerTask("IndexerTask", {
      family: name("indexer"),
      repository: indexerRepository,
      command: [],
      logGroup,
      environment: {
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
      cpu: 512,
      memoryLimitMiB: 1024
    });

    searchTask.addContainer("SearchApi", {
      image: ecs.ContainerImage.fromEcrRepository(searchRepository, "latest"),
      logging: ecs.LogDrivers.awsLogs({
        streamPrefix: "search-api",
        logGroup
      }),
      environment: {
        ARXIVIST_DATA_BUCKET: dataBucket.bucketName,
        ARXIVIST_ACTIVE_INDEX_KEY: "indexes/active/index.json",
        ARXIVIST_CORS_ORIGIN: demoCorsOrigin
      },
      command: ["--bind", "0.0.0.0:3000"],
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
      interval: Duration.seconds(30)
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
      logGroup: logs.ILogGroup;
      environment: Record<string, string>;
    }
  ): ecs.FargateTaskDefinition {
    const task = new ecs.FargateTaskDefinition(this, id, {
      family: props.family,
      cpu: 512,
      memoryLimitMiB: 1024
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
