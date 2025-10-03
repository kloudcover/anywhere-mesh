import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as ecs from 'aws-cdk-lib/aws-ecs';
import * as elbv2 from 'aws-cdk-lib/aws-elasticloadbalancingv2';
import * as route53 from 'aws-cdk-lib/aws-route53';
import * as targets from 'aws-cdk-lib/aws-route53-targets';
import * as acm from 'aws-cdk-lib/aws-certificatemanager';
import * as ecs_patterns from 'aws-cdk-lib/aws-ecs-patterns';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as servicediscovery from 'aws-cdk-lib/aws-servicediscovery';
import * as iam from 'aws-cdk-lib/aws-iam';

export interface InfraStackProps extends cdk.StackProps {
  hostedZoneId: string;
  zoneName: string;
  domainName: string;
  vpcId: string;
  subnetId: string;
  clusterName: string;
  allowedRoleArns: string;
  serviceConnectNamespace: string;
}

export class InfraStack extends cdk.Stack {
  public readonly albDnsName: cdk.CfnOutput;
  public readonly clusterName: cdk.CfnOutput;

  constructor(scope: Construct, id: string, props: InfraStackProps) {
    super(scope, id, props);

    const isInactive = this.node.tryGetContext('inactive') === "true";

    // Use existing VPC
    const vpc = ec2.Vpc.fromLookup(this, 'ExistingVpc', {
      vpcId: props.vpcId
    });

    const execCommandLogGroup = new logs.LogGroup(this, 'ExecCommandLogGroup', {
      logGroupName: `/ecs/${this.stackName}/exec`,
      retention: logs.RetentionDays.ONE_WEEK,
      removalPolicy: cdk.RemovalPolicy.RETAIN
    });

    // Create ECS cluster
    const cluster = new ecs.Cluster(this, 'E2EMeshCluster', {
      vpc,
      clusterName: props.clusterName,
      enableFargateCapacityProviders: true,
      executeCommandConfiguration: {
        logging: ecs.ExecuteCommandLogging.OVERRIDE,
        logConfiguration: {
          cloudWatchLogGroup: execCommandLogGroup
        }
      }
    });

    // Create a unique Service Connect Cloud Map namespace to avoid conflicts with any existing one
    const serviceConnectNamespace = cluster.addDefaultCloudMapNamespace({
      name: props.serviceConnectNamespace,
      vpc,
      useForServiceConnect: true
    });
    // Route53 setup for DNS
    const hostedZone = route53.HostedZone.fromHostedZoneAttributes(this, 'HostedZone', {
      hostedZoneId: props.hostedZoneId,
      zoneName: props.zoneName
    });

    // Certificate for HTTPS (if needed in future)
    const certificate = new acm.Certificate(
      this,
      'Certificate',
      {
        domainName: props.domainName,
        validation: acm.CertificateValidation.fromDns(hostedZone),
        subjectAlternativeNames: [`*.${props.domainName}`]
      }
    )

    // Security group for ALB
    const albSecurityGroup = new ec2.SecurityGroup(this, 'AlbSecurityGroup', {
      vpc,
      description: 'Security group for E2E test ALB',
      allowAllOutbound: true
    });

    // Allow HTTP and WebSocket traffic from internet
    albSecurityGroup.addIngressRule(
      ec2.Peer.anyIpv4(),
      ec2.Port.tcp(443),
      'Allow HTTP from internet'
    );
    albSecurityGroup.addIngressRule(
      ec2.Peer.anyIpv4(),
      ec2.Port.tcp(8082),
      'Allow WebSocket from internet'
    );

    // Security group for ECS tasks
    const taskSecurityGroup = new ec2.SecurityGroup(this, 'TaskSecurityGroup', {
      vpc,
      description: 'Security group for E2E test ECS tasks'
    });

    const meshLogGroup = new logs.LogGroup(this, 'MeshServerLogGroup', {
      logGroupName: `/ecs/${this.stackName}/mesh-server`,
      retention: logs.RetentionDays.ONE_WEEK,
      removalPolicy: cdk.RemovalPolicy.RETAIN
    });

    const serviceConnectTestLogGroup = new logs.LogGroup(this, 'ServiceConnectTestLogGroup', {
      logGroupName: `/ecs/${this.stackName}/service-connect-test`,
      retention: logs.RetentionDays.ONE_WEEK,
      removalPolicy: cdk.RemovalPolicy.RETAIN
    });

    // Allow traffic from ALB
    taskSecurityGroup.addIngressRule(
      albSecurityGroup,
      ec2.Port.tcp(8080),
      'Allow HTTP from ALB'
    );
    taskSecurityGroup.addIngressRule(
      albSecurityGroup,
      ec2.Port.tcp(8082),
      'Allow WebSocket from ALB'
    );

    // Allow Service Connect traffic between tasks sharing this security group
    taskSecurityGroup.addIngressRule(
      taskSecurityGroup,
      ec2.Port.tcp(8080),
      'Allow HTTP from Service Connect tasks'
    );
    taskSecurityGroup.addIngressRule(
      taskSecurityGroup,
      ec2.Port.tcp(8082),
      'Allow WebSocket from Service Connect tasks'
    );

    if (!isInactive) {
      // Create ALB
      const alb = new elbv2.ApplicationLoadBalancer(this, 'E2EMeshAlb', {
        vpc,
        securityGroup: albSecurityGroup,
        internetFacing: true
      });

      // HTTP listener (port 80 -> container port 8080)
      const httpsListener = alb.addListener('HttpsListener', {
        port: 443,
        open: false, // We'll configure target groups manually
        protocol: elbv2.ApplicationProtocol.HTTPS,
        certificates: [certificate],
        defaultAction: elbv2.ListenerAction.fixedResponse(503, {
          contentType: 'text/plain',
          messageBody: 'Service Unavailable'
        })
      });

      // WebSocket listener (port 8082 -> container port 8082)
      const wsListener = alb.addListener('WsListener', {
        port: 8082,
        protocol: elbv2.ApplicationProtocol.HTTPS, // WebSocket upgrades over HTTP
        open: false, // We'll configure target groups manually
        certificates: [certificate],
        defaultAction: elbv2.ListenerAction.fixedResponse(503, {
          contentType: 'text/plain',
          messageBody: 'WebSocket Service Unavailable'
        })
      });
      // A record for the ALB
      new route53.ARecord(this, 'AlbAliasRecord', {
        zone: hostedZone,
        recordName: '*',
        target: route53.RecordTarget.fromAlias(new targets.LoadBalancerTarget(alb))
      });

      // Outputs
      this.albDnsName = new cdk.CfnOutput(this, 'AlbDnsName', {
        value: alb.loadBalancerDnsName,
        description: 'DNS name of the ALB for E2E testing'
      });


      let fargateService: ecs_patterns.ApplicationLoadBalancedFargateService | undefined;

      let containerImage = ecs.ContainerImage.fromAsset("../", {
        ignoreMode: cdk.IgnoreMode.DOCKER,
      });

      // Create task definition
      const taskDefinition = new ecs.FargateTaskDefinition(this, 'TaskDef', {
        cpu: 256,
        memoryLimitMiB: 512,
        runtimePlatform: {
          cpuArchitecture: ecs.CpuArchitecture.ARM64,
          operatingSystemFamily: ecs.OperatingSystemFamily.LINUX
        }
      });

      taskDefinition.addToTaskRolePolicy(new iam.PolicyStatement({
        effect: iam.Effect.ALLOW,
        actions: [
          'ssmmessages:CreateControlChannel',
          'ssmmessages:CreateDataChannel',
          'ssmmessages:OpenControlChannel',
          'ssmmessages:OpenDataChannel'
        ],
        resources: ['*']
      }));

      // Add IAM permissions for Service Connect and Cloud Map
      taskDefinition.addToTaskRolePolicy(new iam.PolicyStatement({
        effect: iam.Effect.ALLOW,
        actions: [
          'servicediscovery:RegisterInstance',
          'servicediscovery:DeregisterInstance',
          'servicediscovery:DiscoverInstances',
          'servicediscovery:GetInstancesHealthStatus',
          'servicediscovery:UpdateInstanceCustomHealthStatus',
          'servicediscovery:GetInstance',
          'servicediscovery:GetService',
          'servicediscovery:ListServices',
          'servicediscovery:ListInstances'
        ],
        resources: ['*']
      }));

      taskDefinition.addContainer('mesh-server', {
        image: containerImage,
        command: ['server', '--alb-port', '8080', '--websocket-port', '8082'],
        environment: {
          RUST_LOG: 'info',
          CLUSTER_NAME: props.clusterName,
          AWS_REGION: this.region,
          // SKIP_IAM_VALIDATION: 'true', // Temporarily disable for testing
          ALLOWED_ROLE_ARNS: props.allowedRoleArns,
        },
        portMappings: [
          { 
            containerPort: 8080, 
            protocol: ecs.Protocol.TCP,
            name: 'mesh-ingress'
          },
          { containerPort: 8082, protocol: ecs.Protocol.TCP }
        ],
        logging: ecs.LogDriver.awsLogs({
          streamPrefix: 'mesh',
          logGroup: meshLogGroup
        })
      });

      // Create Fargate service with Service Connect enabled
      const service = new ecs.FargateService(this, 'Service', {
        cluster,
        taskDefinition,
        desiredCount: 1,
        securityGroups: [taskSecurityGroup],
        assignPublicIp: true, // Enable public IP for ECR access
        vpcSubnets: {
          subnets: [ec2.Subnet.fromSubnetId(this, 'PublicSubnet', props.subnetId)],
        },
        enableExecuteCommand: true,
        serviceConnectConfiguration: {
          namespace: props.serviceConnectNamespace,
          services: [{
            portMappingName: 'mesh-ingress',
            dnsName: 'mesh-ingress',
            port: 8080
          }]
        }
      });

      // Create custom target groups
      const httpTargetGroup = new elbv2.ApplicationTargetGroup(this, 'HttpTargetGroup', {
        vpc,
        port: 8080,
        protocol: elbv2.ApplicationProtocol.HTTP,
        targetType: elbv2.TargetType.IP,
        healthCheck: {
          path: '/health',
          interval: cdk.Duration.seconds(30),
          timeout: cdk.Duration.seconds(5),
          healthyThresholdCount: 2,
          unhealthyThresholdCount: 2
        }
      });

      const wsTargetGroup = new elbv2.ApplicationTargetGroup(this, 'WsTargetGroup', {
        vpc,
        port: 8082,
        protocol: elbv2.ApplicationProtocol.HTTP, // ALB doesn't support WS protocol, use HTTP
        targetType: elbv2.TargetType.IP,
        healthCheck: {
          path: '/health',
          interval: cdk.Duration.seconds(30),
          timeout: cdk.Duration.seconds(5),
          healthyThresholdCount: 2,
          unhealthyThresholdCount: 2
        }
      });

      // Add targets to target groups
      httpTargetGroup.addTarget(service);
      wsTargetGroup.addTarget(service.loadBalancerTarget({
        containerName: 'mesh-server',
        containerPort: 8082
      }));

      // Configure listeners to use target groups
      httpsListener.addTargetGroups('HttpTargetGroup', {
        targetGroups: [httpTargetGroup]
      });

      // Configure WebSocket listener with host-based routing to only allow *.e2e.clustermaestro.com
      wsListener.addAction('WsHostBasedRouting', {
        priority: 100,
        conditions: [
          elbv2.ListenerCondition.hostHeaders(['*.e2e.clustermaestro.com'])
        ],
        action: elbv2.ListenerAction.forward([wsTargetGroup])
      });
      new cdk.CfnOutput(this, 'ServiceName', {
        value: service.serviceName,
        description: 'Name of the ECS service'
      });

      // Create a test container to validate Service Connect routing
      const testTaskDefinition = new ecs.FargateTaskDefinition(this, 'TestTaskDef', {
        cpu: 256,
        memoryLimitMiB: 512,
        runtimePlatform: {
          cpuArchitecture: ecs.CpuArchitecture.ARM64,
          operatingSystemFamily: ecs.OperatingSystemFamily.LINUX
        }
      });

      testTaskDefinition.addToTaskRolePolicy(new iam.PolicyStatement({
        effect: iam.Effect.ALLOW,
        actions: [
          'ssmmessages:CreateControlChannel',
          'ssmmessages:CreateDataChannel',
          'ssmmessages:OpenControlChannel',
          'ssmmessages:OpenDataChannel'
        ],
        resources: ['*']
      }));

      // Add a simple test container that can make HTTP requests
      testTaskDefinition.addContainer('test-client', {
        image: ecs.ContainerImage.fromRegistry('curlimages/curl:latest'),
        command: [
          '/bin/sh', '-c',
          // Keep container running and log Service Connect DNS resolution
          `echo "Service Connect Test Container Started" && 
           echo "Testing DNS resolution for service connect short name mesh-ingress..." &&
           nslookup mesh-ingress || echo "DNS resolution failed" &&
           echo "Testing HTTP connectivity to mesh-ingress:8080..." &&
           while true; do 
             curl -v http://mesh-ingress:8080/health || echo "HTTP request failed";
             echo "Sleeping for 60 seconds...";
             sleep 60;
           done`
        ],
        logging: ecs.LogDriver.awsLogs({
          streamPrefix: 'service-connect-test',
          logGroup: serviceConnectTestLogGroup
        })
      });

      // Create test service with Service Connect enabled
      const testService = new ecs.FargateService(this, 'TestService', {
        cluster,
        taskDefinition: testTaskDefinition,
        desiredCount: 1,
        securityGroups: [taskSecurityGroup],
        assignPublicIp: true,
        vpcSubnets: {
          subnets: [ec2.Subnet.fromSubnetId(this, 'TestPublicSubnet', props.subnetId)],
        },
        enableExecuteCommand: true,
        serviceConnectConfiguration: {
          namespace: props.serviceConnectNamespace,
          // This service only acts as a client, no services exposed
          services: []
        }
      });

      // Skipping explicit Cloud Map service creation; Service Connect will manage
      // Cloud Map service registrations for ECS services automatically.

      new cdk.CfnOutput(this, 'TestServiceName', {
        value: testService.serviceName,
        description: 'Name of the test service for Service Connect validation'
      });

      new cdk.CfnOutput(this, 'ServiceConnectNamespace', {
        value: props.serviceConnectNamespace,
        description: 'Service Connect namespace for hybrid routing'
      });
    }



    this.clusterName = new cdk.CfnOutput(this, 'ClusterName', {
      value: cluster.clusterName,
      description: 'Name of the ECS cluster'
    });
  }
}
