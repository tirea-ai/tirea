"use strict";
var __create = Object.create;
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __getProtoOf = Object.getPrototypeOf;
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
var __toESM = (mod, isNodeMode, target) => (target = mod != null ? __create(__getProtoOf(mod)) : {}, __copyProps(
  // If the importer is in node compatibility mode or this is not an ESM
  // file that has been converted to a CommonJS file using a Babel-
  // compatible transform (i.e. "__esModule" has not been set), then set
  // "default" to the CommonJS "module.exports" for node compatibility.
  isNodeMode || !mod || !mod.__esModule ? __defProp(target, "default", { value: mod, enumerable: true }) : target,
  mod
));
var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);

// src/components/chat/Button.tsx
var Button_exports = {};
__export(Button_exports, {
  Button: () => Button
});
module.exports = __toCommonJS(Button_exports);

// src/components/chat/ChatContext.tsx
var import_react = __toESM(require("react"));
var import_jsx_runtime = require("react/jsx-runtime");
var ChatContext = import_react.default.createContext(void 0);
function useChatContext() {
  const context = import_react.default.useContext(ChatContext);
  if (context === void 0) {
    throw new Error(
      "Context not found. Did you forget to wrap your app in a <ChatContextProvider> component?"
    );
  }
  return context;
}

// src/components/chat/Button.tsx
var import_jsx_runtime2 = require("react/jsx-runtime");
var Button = ({}) => {
  const { open, setOpen, icons } = useChatContext();
  return /* @__PURE__ */ (0, import_jsx_runtime2.jsx)("div", { onClick: () => setOpen(!open), children: /* @__PURE__ */ (0, import_jsx_runtime2.jsxs)(
    "button",
    {
      className: `copilotKitButton ${open ? "open" : ""}`,
      "aria-label": open ? "Close Chat" : "Open Chat",
      children: [
        /* @__PURE__ */ (0, import_jsx_runtime2.jsx)("div", { className: "copilotKitButtonIcon copilotKitButtonIconOpen", children: icons.openIcon }),
        /* @__PURE__ */ (0, import_jsx_runtime2.jsx)("div", { className: "copilotKitButtonIcon copilotKitButtonIconClose", children: icons.closeIcon })
      ]
    }
  ) });
};
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  Button
});
//# sourceMappingURL=Button.js.map