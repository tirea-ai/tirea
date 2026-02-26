"use strict";
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __export = (target, all) => {
  for (var name in all)
    __defProp(target, name, { get: all[name], enumerable: true });
};
var __copyProps = (to, from, except, desc) => {
  if (from && typeof from === "object" || typeof from === "function") {
    for (let key of __getOwnPropNames(from))
      if (!__hasOwnProp.call(to, key) && key !== except)
        __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
  }
  return to;
};
var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);
var __async = (__this, __arguments, generator) => {
  return new Promise((resolve, reject) => {
    var fulfilled = (value) => {
      try {
        step(generator.next(value));
      } catch (e) {
        reject(e);
      }
    };
    var rejected = (value) => {
      try {
        step(generator.throw(value));
      } catch (e) {
        reject(e);
      }
    };
    var step = (x) => x.done ? resolve(x.value) : Promise.resolve(x.value).then(fulfilled, rejected);
    step((generator = generator.apply(__this, __arguments)).next());
  });
};

// src/lib/status-checker.ts
var status_checker_exports = {};
__export(status_checker_exports, {
  StatusChecker: () => StatusChecker
});
module.exports = __toCommonJS(status_checker_exports);
var import_shared = require("@copilotkit/shared");
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
          const response = yield fetch(`${import_shared.COPILOT_CLOUD_API_URL}/ciu`, {
            method: "GET",
            headers: {
              [import_shared.COPILOT_CLOUD_PUBLIC_API_KEY_HEADER]: publicApiKey
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
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  StatusChecker
});
//# sourceMappingURL=status-checker.js.map