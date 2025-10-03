import http from "k6/http";
import { check, sleep } from "k6";

export const options = {
  vus: 1,
  duration: "10s",
  thresholds: {
    http_req_failed: ["rate<0.01"],
    http_req_duration: ["p(95)<800"],
  },
  tags: { suite: "smoke" },
};

const TARGET_HOST = __ENV.TARGET_HOST || "localhost:3000";
const BASE_URL = __ENV.BASE_URL || `https://${TARGET_HOST}`;
console.log("TARGET_HOST", TARGET_HOST);

export default function () {
  const params = { headers: { Host: TARGET_HOST } };

  const res1 = http.get(`${BASE_URL}/`, params);
  check(res1, { "health 200": (r) => r.status === 200 });

  sleep(0.5);
}
