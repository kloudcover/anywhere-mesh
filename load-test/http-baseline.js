import http from 'k6/http';
import { check, sleep } from 'k6';

export const options = {
  scenarios: {
    baseline_blah: {
      executor: 'ramping-arrival-rate',
      startRate: 5,
      timeUnit: '1s',
      preAllocatedVUs: 20,
      maxVUs: 100,
      stages: [
        { duration: '1m', target: 20 },
        { duration: '1m', target: 20 },
        { duration: '1m', target: 0 },
      ],
      env: { TARGET_HOST: 'blah.kloudcover.com' },
      tags: { endpoint: 'blah' },
    },
    baseline_test: {
      executor: 'ramping-arrival-rate',
      startRate: 5,
      timeUnit: '1s',
      preAllocatedVUs: 20,
      maxVUs: 100,
      stages: [
        { duration: '1m', target: 20 },
        { duration: '1m', target: 20 },
        { duration: '1m', target: 0 },
      ],
      env: { TARGET_HOST: 'test.kloudcover.com' },
      tags: { endpoint: 'test' },
    },
  },
  thresholds: {
    http_req_failed: ['rate<0.01'],
    http_req_duration: ['p(95)<500'],
  },
  tags: { suite: 'baseline' },
};

const paths = ['/api/health', '/api/info', '/api/echo?message=hello', '/api/time'];

export default function () {
  // Read TARGET_HOST from scenario environment for each VU
  const TARGET_HOST = __ENV.TARGET_HOST;
  const BASE_URL = __ENV.BASE_URL || `https://${TARGET_HOST}`;

  const path = paths[Math.floor(Math.random() * paths.length)];
  const params = { headers: { Host: TARGET_HOST, 'X-Test-Route': 'k6-baseline' } };

  const res = http.get(`${BASE_URL}${path}`, params);
  check(res, {
    'status is 200': (r) => r.status === 200,
    'ttfb < 200ms': (r) => r.timings.waiting < 200,
  });

  sleep(0.1);
}


