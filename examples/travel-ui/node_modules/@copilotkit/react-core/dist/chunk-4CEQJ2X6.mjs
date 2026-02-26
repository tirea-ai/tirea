// src/types/frontend-action.ts
import { ActionInputAvailability } from "@copilotkit/runtime-client-gql";
import {
  actionParametersToJsonSchema
} from "@copilotkit/shared";
function processActionsForRuntimeRequest(actions) {
  const filteredActions = actions.filter(
    (action) => action.available !== ActionInputAvailability.Disabled && action.disabled !== true && action.name !== "*" && action.available != "frontend" && !action.pairedAction
  ).map((action) => {
    let available = ActionInputAvailability.Enabled;
    if (action.disabled) {
      available = ActionInputAvailability.Disabled;
    } else if (action.available === "disabled") {
      available = ActionInputAvailability.Disabled;
    } else if (action.available === "remote") {
      available = ActionInputAvailability.Remote;
    }
    return {
      name: action.name,
      description: action.description || "",
      jsonSchema: JSON.stringify(actionParametersToJsonSchema(action.parameters || [])),
      available
    };
  });
  return filteredActions;
}

export {
  processActionsForRuntimeRequest
};
//# sourceMappingURL=chunk-4CEQJ2X6.mjs.map