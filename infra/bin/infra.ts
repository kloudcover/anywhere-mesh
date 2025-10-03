#!/usr/bin/env node
import * as cdk from 'aws-cdk-lib';
import { InfraStack, InfraStackProps } from '../lib/stack';
import * as fs from 'fs';

const app = new cdk.App();

const paramsPath = './params.json';
if (!fs.existsSync(paramsPath)) {
  throw new Error('params.json not found. Please copy params.example.json to params.json and fill in your values.');
}
const params = JSON.parse(fs.readFileSync(paramsPath, 'utf-8'));

new InfraStack(app, 'ecs-anywhere-mesh-e2e', {
  env: {
    account: params.account,
    region: params.region || process.env.CDK_DEFAULT_REGION || 'us-west-2'
  },
  hostedZoneId: params.hostedZoneId,
  zoneName: params.zoneName,
  domainName: params.domainName,
  vpcId: params.vpcId,
  subnetId: params.subnetId,
  clusterName: params.clusterName,
  allowedRoleArns: params.allowedRoleArns,
  serviceConnectNamespace: params.serviceConnectNamespace
} as InfraStackProps);