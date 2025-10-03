import ws from "k6/ws";
import { check, sleep } from "k6";

export const options = {
  scenarios: {
    ws_burst: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [{ duration: "10s", target: 50 }],
    },
  },
  thresholds: {
    checks: ["rate>0.95"],
  },
  tags: { suite: "ws" },
};

const TARGET_HOST = __ENV.TARGET_HOST || "localhost:3000";
const WS_URL = __ENV.WS_URL || `wss://${TARGET_HOST}`;

export default function () {
  const url = `${WS_URL}/ws`; // WebSocket endpoint
  const params = { headers: { Host: TARGET_HOST } };

  const res = ws.connect(url, params, function (socket) {
    socket.on("open", function () {
      socket.send("ping");
    });

    socket.on("message", function () {
      // No echo expected by server; this validates upgrade & stability
    });

    socket.on("error", function () {});

    sleep(5);
    socket.close();
  });

  check(res, { connected: (r) => r && r.status === 101 });
}
