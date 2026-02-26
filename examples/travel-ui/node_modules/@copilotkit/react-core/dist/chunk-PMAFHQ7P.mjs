import {
  __async
} from "./chunk-SKC7AJIV.mjs";

// src/lib/status-checker.ts
import {
  COPILOT_CLOUD_API_URL,
  COPILOT_CLOUD_PUBLIC_API_KEY_HEADER
} from "@copilotkit/shared";
var STATUS_CHECK_INTERVAL = 1e3 * 60 * 5;
var StatusChecker = class {
  constructor() {
    this.activeKey = null;
    this.intervalId = null;
    this.instanceCount = 0;
    this.lastResponse = null;
  }
  start(publicApiKey, onUpdate) {
    return __async(this, null, function* () {
      this.instanceCount++;
      if (this.activeKey === publicApiKey)
        return;
      if (this.intervalId)
        clearInterval(this.intervalId);
      const checkStatus = () => __async(this, null, function* () {
        try {
          const response = yield fetch(`${COPILOT_CLOUD_API_URL}/ciu`, {
            method: "GET",
            headers: {
              [COPILOT_CLOUD_PUBLIC_API_KEY_HEADER]: publicApiKey
            }
          }).then((response2) => response2.json());
          this.lastResponse = response;
          onUpdate == null ? void 0 : onUpdate(response);
          return response;
        } catch (error) {
          return null;
        }
      });
      const initialResponse = yield checkStatus();
      this.intervalId = setInterval(checkStatus, STATUS_CHECK_INTERVAL);
      this.activeKey = publicApiKey;
      return initialResponse;
    });
  }
  getLastResponse() {
    return this.lastResponse;
  }
  stop() {
    this.instanceCount--;
    if (this.instanceCount === 0) {
      if (this.intervalId) {
        clearInterval(this.intervalId);
        this.intervalId = null;
        this.activeKey = null;
        this.lastResponse = null;
      }
    }
  }
};

export {
  StatusChecker
};
//# sourceMappingURL=chunk-PMAFHQ7P.mjs.map