import * as cdk from "aws-cdk-lib";
import * as cloudfront from "aws-cdk-lib/aws-cloudfront";
import * as origins from "aws-cdk-lib/aws-cloudfront-origins";
import * as ec2 from "aws-cdk-lib/aws-ec2";
import * as ecrAssets from "aws-cdk-lib/aws-ecr-assets";
import * as ecs from "aws-cdk-lib/aws-ecs";
import * as efs from "aws-cdk-lib/aws-efs";
import * as elbv2 from "aws-cdk-lib/aws-elasticloadbalancingv2";
import * as logs from "aws-cdk-lib/aws-logs";
import * as s3 from "aws-cdk-lib/aws-s3";
import * as secretsmanager from "aws-cdk-lib/aws-secretsmanager";
import * as servicediscovery from "aws-cdk-lib/aws-servicediscovery";
import { Construct } from "constructs";
import * as path from "node:path";

export interface ArxivistStackProps extends cdk.StackProps { releaseId: string }

export class ArxivistStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props: ArxivistStackProps) {
    super(scope, id, props);
    const artifacts = new s3.Bucket(this, "Artifacts", {
      encryption: s3.BucketEncryption.S3_MANAGED,
      blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
      versioned: true,
      enforceSSL: true,
      removalPolicy: cdk.RemovalPolicy.RETAIN,
      lifecycleRules: [{ noncurrentVersionExpiration: cdk.Duration.days(30) }],
    });
    const proxySecret = new secretsmanager.Secret(this, "ProxySecret", {
      secretName: "arxivist/proxy-shared-secret",
      description: "Shared Vercel-to-Arxivist API proxy credential.",
      generateSecretString: { excludePunctuation: true },
    });
    const openAiKey = new secretsmanager.Secret(this, "OpenAiKey", {
      secretName: "arxivist/openai-api-key",
      description: "Replace this generated value with the raw OpenAI API key before enabling agent search.",
      generateSecretString: { excludePunctuation: true },
    });

    const vpc = new ec2.Vpc(this, "Vpc", { maxAzs: 2, natGateways: 0 });
    const albSg = new ec2.SecurityGroup(this, "AlbSg", { vpc });
    albSg.addIngressRule(ec2.Peer.anyIpv4(), ec2.Port.tcp(80));
    const searchSg = new ec2.SecurityGroup(this, "SearchSg", { vpc });
    const agentSg = new ec2.SecurityGroup(this, "AgentSg", { vpc });
    const syncSg = new ec2.SecurityGroup(this, "ArtifactSyncSg", { vpc });
    searchSg.addIngressRule(albSg, ec2.Port.tcp(3000));
    searchSg.addIngressRule(agentSg, ec2.Port.tcp(3000));
    agentSg.addIngressRule(albSg, ec2.Port.tcp(8000));

    const fileSystem = new efs.FileSystem(this, "DataFileSystem", {
      vpc, encrypted: true, removalPolicy: cdk.RemovalPolicy.RETAIN,
      vpcSubnets: { subnetType: ec2.SubnetType.PRIVATE_ISOLATED },
    });
    fileSystem.connections.allowDefaultPortFrom(searchSg);
    fileSystem.connections.allowDefaultPortFrom(agentSg);
    fileSystem.connections.allowDefaultPortFrom(syncSg);
    const data = fileSystem.addAccessPoint("DataAccess", {
      path: "/arxivist",
      createAcl: { ownerGid: "1000", ownerUid: "1000", permissions: "750" },
      posixUser: { gid: "1000", uid: "1000" },
    });

    const cluster = new ecs.Cluster(this, "Cluster", { vpc });
    const namespace = new servicediscovery.PrivateDnsNamespace(this, "Namespace", { vpc, name: "arxivist.local" });
    const logsGroup = new logs.LogGroup(this, "Logs", { retention: logs.RetentionDays.TWO_WEEKS, removalPolicy: cdk.RemovalPolicy.RETAIN });
    const root = path.resolve(import.meta.dirname, "../..");
    const searchImage = new ecrAssets.DockerImageAsset(this, "SearchImage", { directory: root, file: "Dockerfile", buildArgs: { BIN: "arxivist-search-api" } });
    const agentImage = new ecrAssets.DockerImageAsset(this, "AgentImage", { directory: root, file: "arxivist-agent/Dockerfile" });

    const searchTask = this.task("SearchTask", data);
    const search = searchTask.addContainer("Search", {
      image: ecs.ContainerImage.fromDockerImageAsset(searchImage),
      logging: ecs.LogDrivers.awsLogs({ logGroup: logsGroup, streamPrefix: "search" }),
      command: ["--index", `/data/releases/${props.releaseId}/index`, "--bind", "0.0.0.0:3000"],
      secrets: { ARXIVIST_PROXY_SHARED_SECRET: ecs.Secret.fromSecretsManager(proxySecret) },
    });
    search.addPortMappings({ containerPort: 3000 });
    search.addMountPoints({ sourceVolume: "data", containerPath: "/data", readOnly: true });

    const agentTask = this.task("AgentTask", data);
    const agent = agentTask.addContainer("Agent", {
      image: ecs.ContainerImage.fromDockerImageAsset(agentImage),
      logging: ecs.LogDrivers.awsLogs({ logGroup: logsGroup, streamPrefix: "agent" }),
      environment: {
        ARXIVIST_SEARCH_API_BASE_URL: "http://search.arxivist.local:3000",
        ARXIVIST_LOCAL_DATA_DIR: `/data/releases/${props.releaseId}/crawl`,
        ARXIVIST_OSM_USER_AGENT: "Arxivist/production",
      },
      secrets: {
        OPENAI_API_KEY: ecs.Secret.fromSecretsManager(openAiKey),
        ARXIVIST_PROXY_SHARED_SECRET: ecs.Secret.fromSecretsManager(proxySecret),
      },
    });
    agent.addPortMappings({ containerPort: 8000 });
    agent.addMountPoints({ sourceVolume: "data", containerPath: "/data", readOnly: true });

    const searchService = new ecs.FargateService(this, "SearchService", {
      cluster, taskDefinition: searchTask, desiredCount: 1, assignPublicIp: true,
      securityGroups: [searchSg], vpcSubnets: { subnetType: ec2.SubnetType.PUBLIC },
      cloudMapOptions: { cloudMapNamespace: namespace, name: "search" },
    });
    const agentService = new ecs.FargateService(this, "AgentService", {
      cluster, taskDefinition: agentTask, desiredCount: 1, assignPublicIp: true,
      securityGroups: [agentSg], vpcSubnets: { subnetType: ec2.SubnetType.PUBLIC },
    });
    const alb = new elbv2.ApplicationLoadBalancer(this, "ApiAlb", { vpc, internetFacing: true, securityGroup: albSg });
    const listener = alb.addListener("Http", { port: 80, open: true });
    listener.addTargets("SearchTargets", {
      priority: 10, conditions: [elbv2.ListenerCondition.pathPatterns(["/search"])], port: 3000,
      targets: [searchService], healthCheck: { path: "/health" },
    });
    listener.addTargets("AgentTargets", {
      priority: 20, conditions: [elbv2.ListenerCondition.pathPatterns(["/agent/*", "/places/*"])], port: 8000,
      targets: [agentService], healthCheck: { path: "/health" },
    });
    listener.addAction("NotFound", { action: elbv2.ListenerAction.fixedResponse(404) });
    // CloudFront supplies a TLS endpoint even before a custom domain is added.
    const apiDistribution = new cloudfront.Distribution(this, "ApiDistribution", {
      defaultBehavior: {
        origin: new origins.HttpOrigin(alb.loadBalancerDnsName, { protocolPolicy: cloudfront.OriginProtocolPolicy.HTTP_ONLY }),
        viewerProtocolPolicy: cloudfront.ViewerProtocolPolicy.REDIRECT_TO_HTTPS,
        cachePolicy: cloudfront.CachePolicy.CACHING_DISABLED,
        originRequestPolicy: cloudfront.OriginRequestPolicy.ALL_VIEWER_EXCEPT_HOST_HEADER,
        allowedMethods: cloudfront.AllowedMethods.ALLOW_ALL,
      },
    });

    const syncImage = new ecrAssets.DockerImageAsset(this, "SyncImage", { directory: path.join(root, "infra"), file: "Dockerfile.sync" });
    const syncTask = this.task("ArtifactSyncTask", data);
    artifacts.grantRead(syncTask.taskRole);
    const sync = syncTask.addContainer("ArtifactSync", {
      image: ecs.ContainerImage.fromDockerImageAsset(syncImage),
      logging: ecs.LogDrivers.awsLogs({ logGroup: logsGroup, streamPrefix: "artifact-sync" }),
      environment: { ARXIVIST_ARTIFACT_BUCKET: artifacts.bucketName },
    });
    sync.addMountPoints({ sourceVolume: "data", containerPath: "/data", readOnly: false });

    new cdk.CfnOutput(this, "ArtifactBucketName", { value: artifacts.bucketName });
    new cdk.CfnOutput(this, "LoadBalancerUrl", { value: `http://${alb.loadBalancerDnsName}` });
    new cdk.CfnOutput(this, "ApiOriginUrl", { value: `https://${apiDistribution.distributionDomainName}` });
    new cdk.CfnOutput(this, "ArtifactSyncTaskDefinitionArn", { value: syncTask.taskDefinitionArn });
    new cdk.CfnOutput(this, "ArtifactSyncSecurityGroupId", { value: syncSg.securityGroupId });
    new cdk.CfnOutput(this, "ProxySecretArn", { value: proxySecret.secretArn });
    new cdk.CfnOutput(this, "OpenAiSecretArn", { value: openAiKey.secretArn });
  }

  private task(id: string, accessPoint: efs.AccessPoint): ecs.FargateTaskDefinition {
    const task = new ecs.FargateTaskDefinition(this, id, { cpu: 1024, memoryLimitMiB: 2048 });
    task.addVolume({ name: "data", efsVolumeConfiguration: {
      fileSystemId: accessPoint.fileSystem.fileSystemId,
      transitEncryption: "ENABLED",
      authorizationConfig: { accessPointId: accessPoint.accessPointId },
    }});
    return task;
  }
}
