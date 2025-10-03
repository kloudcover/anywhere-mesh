import http from 'k6/http';
import { check, sleep } from 'k6';

export const options = {
  scenarios: {
    soak: {
      executor: 'constant-arrival-rate',
      rate: 20, // requests per second
      timeUnit: '1s',
      duration: '30m',
      preAllocatedVUs: 50,
      maxVUs: 200,
    },
  },
  thresholds: {
    http_req_failed: ['rate<0.02'],
    http_req_duration: ['p(99)<1000'],
  },
  tags: { suite: 'soak' },
};

const TARGET_HOST = __ENV.TARGET_HOST || 'blah.kloudcover.com';
const BASE_URL = __ENV.BASE_URL || `https://${TARGET_HOST}`;

export default function () {
  const delay = Math.floor(Math.random() * 3); // <= 2s
  const params = { headers: { Host: TARGET_HOST, 'X-Test-Route': 'k6-soak' } };
  const res = http.get(`${BASE_URL}/api/stress/${delay}`, params);
  check(res, { '200': (r) => r.status === 200 });
  sleep(0.1);
}


