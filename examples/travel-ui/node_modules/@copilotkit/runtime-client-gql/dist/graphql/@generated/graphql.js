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

// src/graphql/@generated/graphql.ts
var graphql_exports = {};
__export(graphql_exports, {
  ActionInputAvailability: () => ActionInputAvailability,
  AvailableAgentsDocument: () => AvailableAgentsDocument,
  CopilotRequestType: () => CopilotRequestType,
  FailedResponseStatusReason: () => FailedResponseStatusReason,
  GenerateCopilotResponseDocument: () => GenerateCopilotResponseDocument,
  LoadAgentStateDocument: () => LoadAgentStateDocument,
  MessageRole: () => MessageRole,
  MessageStatusCode: () => MessageStatusCode,
  MetaEventName: () => MetaEventName,
  ResponseStatusCode: () => ResponseStatusCode
});
module.exports = __toCommonJS(graphql_exports);
var ActionInputAvailability = /* @__PURE__ */ ((ActionInputAvailability2) => {
  ActionInputAvailability2["Disabled"] = "disabled";
  ActionInputAvailability2["Enabled"] = "enabled";
  ActionInputAvailability2["Remote"] = "remote";
  return ActionInputAvailability2;
})(ActionInputAvailability || {});
var CopilotRequestType = /* @__PURE__ */ ((CopilotRequestType2) => {
  CopilotRequestType2["Chat"] = "Chat";
  CopilotRequestType2["Suggestion"] = "Suggestion";
  CopilotRequestType2["Task"] = "Task";
  CopilotRequestType2["TextareaCompletion"] = "TextareaCompletion";
  CopilotRequestType2["TextareaPopover"] = "TextareaPopover";
  return CopilotRequestType2;
})(CopilotRequestType || {});
var FailedResponseStatusReason = /* @__PURE__ */ ((FailedResponseStatusReason2) => {
  FailedResponseStatusReason2["GuardrailsValidationFailed"] = "GUARDRAILS_VALIDATION_FAILED";
  FailedResponseStatusReason2["MessageStreamInterrupted"] = "MESSAGE_STREAM_INTERRUPTED";
  FailedResponseStatusReason2["UnknownError"] = "UNKNOWN_ERROR";
  return FailedResponseStatusReason2;
})(FailedResponseStatusReason || {});
var MessageRole = /* @__PURE__ */ ((MessageRole2) => {
  MessageRole2["Assistant"] = "assistant";
  MessageRole2["Developer"] = "developer";
  MessageRole2["System"] = "system";
  MessageRole2["Tool"] = "tool";
  MessageRole2["User"] = "user";
  return MessageRole2;
})(MessageRole || {});
var MessageStatusCode = /* @__PURE__ */ ((MessageStatusCode2) => {
  MessageStatusCode2["Failed"] = "Failed";
  MessageStatusCode2["Pending"] = "Pending";
  MessageStatusCode2["Success"] = "Success";
  return MessageStatusCode2;
})(MessageStatusCode || {});
var MetaEventName = /* @__PURE__ */ ((MetaEventName2) => {
  MetaEventName2["CopilotKitLangGraphInterruptEvent"] = "CopilotKitLangGraphInterruptEvent";
  MetaEventName2["LangGraphInterruptEvent"] = "LangGraphInterruptEvent";
  return MetaEventName2;
})(MetaEventName || {});
var ResponseStatusCode = /* @__PURE__ */ ((ResponseStatusCode2) => {
  ResponseStatusCode2["Failed"] = "Failed";
  ResponseStatusCode2["Pending"] = "Pending";
  ResponseStatusCode2["Success"] = "Success";
  return ResponseStatusCode2;
})(ResponseStatusCode || {});
var GenerateCopilotResponseDocument = { "kind": "Document", "definitions": [{ "kind": "OperationDefinition", "operation": "mutation", "name": { "kind": "Name", "value": "generateCopilotResponse" }, "variableDefinitions": [{ "kind": "VariableDefinition", "variable": { "kind": "Variable", "name": { "kind": "Name", "value": "data" } }, "type": { "kind": "NonNullType", "type": { "kind": "NamedType", "name": { "kind": "Name", "value": "GenerateCopilotResponseInput" } } } }, { "kind": "VariableDefinition", "variable": { "kind": "Variable", "name": { "kind": "Name", "value": "properties" } }, "type": { "kind": "NamedType", "name": { "kind": "Name", "value": "JSONObject" } } }], "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "generateCopilotResponse" }, "arguments": [{ "kind": "Argument", "name": { "kind": "Name", "value": "data" }, "value": { "kind": "Variable", "name": { "kind": "Name", "value": "data" } } }, { "kind": "Argument", "name": { "kind": "Name", "value": "properties" }, "value": { "kind": "Variable", "name": { "kind": "Name", "value": "properties" } } }], "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "threadId" } }, { "kind": "Field", "name": { "kind": "Name", "value": "runId" } }, { "kind": "Field", "name": { "kind": "Name", "value": "extensions" }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "openaiAssistantAPI" }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "runId" } }, { "kind": "Field", "name": { "kind": "Name", "value": "threadId" } }] } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "CopilotResponse" } }, "directives": [{ "kind": "Directive", "name": { "kind": "Name", "value": "defer" } }], "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "status" }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "BaseResponseStatus" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "code" } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "FailedResponseStatus" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "reason" } }, { "kind": "Field", "name": { "kind": "Name", "value": "details" } }] } }] } }] } }, { "kind": "Field", "name": { "kind": "Name", "value": "messages" }, "directives": [{ "kind": "Directive", "name": { "kind": "Name", "value": "stream" } }], "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "__typename" } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "BaseMessageOutput" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "id" } }, { "kind": "Field", "name": { "kind": "Name", "value": "createdAt" } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "BaseMessageOutput" } }, "directives": [{ "kind": "Directive", "name": { "kind": "Name", "value": "defer" } }], "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "status" }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "SuccessMessageStatus" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "code" } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "FailedMessageStatus" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "code" } }, { "kind": "Field", "name": { "kind": "Name", "value": "reason" } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "PendingMessageStatus" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "code" } }] } }] } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "TextMessageOutput" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "content" }, "directives": [{ "kind": "Directive", "name": { "kind": "Name", "value": "stream" } }] }, { "kind": "Field", "name": { "kind": "Name", "value": "role" } }, { "kind": "Field", "name": { "kind": "Name", "value": "parentMessageId" } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "ImageMessageOutput" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "format" } }, { "kind": "Field", "name": { "kind": "Name", "value": "bytes" } }, { "kind": "Field", "name": { "kind": "Name", "value": "role" } }, { "kind": "Field", "name": { "kind": "Name", "value": "parentMessageId" } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "ActionExecutionMessageOutput" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "name" } }, { "kind": "Field", "name": { "kind": "Name", "value": "arguments" }, "directives": [{ "kind": "Directive", "name": { "kind": "Name", "value": "stream" } }] }, { "kind": "Field", "name": { "kind": "Name", "value": "parentMessageId" } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "ResultMessageOutput" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "result" } }, { "kind": "Field", "name": { "kind": "Name", "value": "actionExecutionId" } }, { "kind": "Field", "name": { "kind": "Name", "value": "actionName" } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "AgentStateMessageOutput" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "threadId" } }, { "kind": "Field", "name": { "kind": "Name", "value": "state" } }, { "kind": "Field", "name": { "kind": "Name", "value": "running" } }, { "kind": "Field", "name": { "kind": "Name", "value": "agentName" } }, { "kind": "Field", "name": { "kind": "Name", "value": "nodeName" } }, { "kind": "Field", "name": { "kind": "Name", "value": "runId" } }, { "kind": "Field", "name": { "kind": "Name", "value": "active" } }, { "kind": "Field", "name": { "kind": "Name", "value": "role" } }] } }] } }, { "kind": "Field", "name": { "kind": "Name", "value": "metaEvents" }, "directives": [{ "kind": "Directive", "name": { "kind": "Name", "value": "stream" } }], "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "LangGraphInterruptEvent" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "type" } }, { "kind": "Field", "name": { "kind": "Name", "value": "name" } }, { "kind": "Field", "name": { "kind": "Name", "value": "value" } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "CopilotKitLangGraphInterruptEvent" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "type" } }, { "kind": "Field", "name": { "kind": "Name", "value": "name" } }, { "kind": "Field", "name": { "kind": "Name", "value": "data" }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "messages" }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "__typename" } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "BaseMessageOutput" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "id" } }, { "kind": "Field", "name": { "kind": "Name", "value": "createdAt" } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "BaseMessageOutput" } }, "directives": [{ "kind": "Directive", "name": { "kind": "Name", "value": "defer" } }], "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "status" }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "SuccessMessageStatus" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "code" } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "FailedMessageStatus" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "code" } }, { "kind": "Field", "name": { "kind": "Name", "value": "reason" } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "PendingMessageStatus" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "code" } }] } }] } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "TextMessageOutput" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "content" } }, { "kind": "Field", "name": { "kind": "Name", "value": "role" } }, { "kind": "Field", "name": { "kind": "Name", "value": "parentMessageId" } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "ActionExecutionMessageOutput" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "name" } }, { "kind": "Field", "name": { "kind": "Name", "value": "arguments" } }, { "kind": "Field", "name": { "kind": "Name", "value": "parentMessageId" } }] } }, { "kind": "InlineFragment", "typeCondition": { "kind": "NamedType", "name": { "kind": "Name", "value": "ResultMessageOutput" } }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "result" } }, { "kind": "Field", "name": { "kind": "Name", "value": "actionExecutionId" } }, { "kind": "Field", "name": { "kind": "Name", "value": "actionName" } }] } }] } }, { "kind": "Field", "name": { "kind": "Name", "value": "value" } }] } }] } }] } }] } }] } }] };
var AvailableAgentsDocument = { "kind": "Document", "definitions": [{ "kind": "OperationDefinition", "operation": "query", "name": { "kind": "Name", "value": "availableAgents" }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "availableAgents" }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "agents" }, "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "name" } }, { "kind": "Field", "name": { "kind": "Name", "value": "id" } }, { "kind": "Field", "name": { "kind": "Name", "value": "description" } }] } }] } }] } }] };
var LoadAgentStateDocument = { "kind": "Document", "definitions": [{ "kind": "OperationDefinition", "operation": "query", "name": { "kind": "Name", "value": "loadAgentState" }, "variableDefinitions": [{ "kind": "VariableDefinition", "variable": { "kind": "Variable", "name": { "kind": "Name", "value": "data" } }, "type": { "kind": "NonNullType", "type": { "kind": "NamedType", "name": { "kind": "Name", "value": "LoadAgentStateInput" } } } }], "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "loadAgentState" }, "arguments": [{ "kind": "Argument", "name": { "kind": "Name", "value": "data" }, "value": { "kind": "Variable", "name": { "kind": "Name", "value": "data" } } }], "selectionSet": { "kind": "SelectionSet", "selections": [{ "kind": "Field", "name": { "kind": "Name", "value": "threadId" } }, { "kind": "Field", "name": { "kind": "Name", "value": "threadExists" } }, { "kind": "Field", "name": { "kind": "Name", "value": "state" } }, { "kind": "Field", "name": { "kind": "Name", "value": "messages" } }] } }] } }] };
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  ActionInputAvailability,
  AvailableAgentsDocument,
  CopilotRequestType,
  FailedResponseStatusReason,
  GenerateCopilotResponseDocument,
  LoadAgentStateDocument,
  MessageRole,
  MessageStatusCode,
  MetaEventName,
  ResponseStatusCode
});
//# sourceMappingURL=graphql.js.map