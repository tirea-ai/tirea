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

// src/types/frontend-action.ts
var frontend_action_exports = {};
__export(frontend_action_exports, {
  processActionsForRuntimeRequest: () => processActionsForRuntimeRequest
});
module.exports = __toCommonJS(frontend_action_exports);
var import_runtime_client_gql = require("@copilotkit/runtime-client-gql");
var import_shared = require("@copilotkit/shared");
function processActionsForRuntimeRequest(actions) {
  const filteredActions = actions.filter(
    (action) => action.available !== import_runtime_client_gql.ActionInputAvailability.Disabled && action.disabled !== true && action.name !== "*" && action.available != "frontend" && !action.pairedAction
  ).map((action) => {
    let available = import_runtime_client_gql.ActionInputAvailability.Enabled;
    if (action.disabled) {
      available = import_runtime_client_gql.ActionInputAvailability.Disabled;
    } else if (action.available === "disabled") {
      available = import_runtime_client_gql.ActionInputAvailability.Disabled;
    } else if (action.available === "remote") {
      available = import_runtime_client_gql.ActionInputAvailability.Remote;
    }
    return {
      name: action.name,
      description: action.description || "",
      jsonSchema: JSON.stringify((0, import_shared.actionParametersToJsonSchema)(action.parameters || [])),
      available
    };
  });
  return filteredActions;
}
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  processActionsForRuntimeRequest
});
//# sourceMappingURL=frontend-action.js.map