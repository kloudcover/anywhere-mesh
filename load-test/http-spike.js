import http from 'k6/http';
import { check, sleep } from 'k6';

export const options = {
  scenarios: {
    spike: {
      executor: 'ramping-arrival-rate',
      startRate: 0,
      timeUnit: '1s',
      preAllocatedVUs: 50,
      maxVUs: 500,
      stages: [
        { duration: '30s', target: 100 },
        { duration: '1m', target: 300 },
        { duration: '1m', target: 50 },
        { duration: '30s', target: 0 },
      ],
    },
  },
  thresholds: {
    http_req_failed: ['rate<0.05'],
    http_req_duration: ['p(95)<1200'],
  },
  tags: { suite: 'spike' },
};

const TARGET_HOST = __ENV.TARGET_HOST || 'blah.kloudcover.com';
const BASE_URL = __ENV.BASE_URL || `https://${TARGET_HOST}`;

export default function () {
  const params = { headers: { Host: TARGET_HOST, 'X-Test-Route': 'k6-spike' } };
  const res = http.get(`${BASE_URL}/api/echo?msg=spike`, params);
  check(res, { '200': (r) => r.status === 200 });
  sleep(0.01);
}


