"use strict";
var __create = Object.create;
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __getOwnPropSymbols = Object.getOwnPropertySymbols;
var __getProtoOf = Object.getPrototypeOf;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __propIsEnum = Object.prototype.propertyIsEnumerable;
var __defNormalProp = (obj, key, value) => key in obj ? __defProp(obj, key, { enumerable: true, configurable: true, writable: true, value }) : obj[key] = value;
var __spreadValues = (a, b) => {
  for (var prop in b || (b = {}))
    if (__hasOwnProp.call(b, prop))
      __defNormalProp(a, prop, b[prop]);
  if (__getOwnPropSymbols)
    for (var prop of __getOwnPropSymbols(b)) {
      if (__propIsEnum.call(b, prop))
        __defNormalProp(a, prop, b[prop]);
    }
  return a;
};
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

// src/components/chat/ChatContext.tsx
var ChatContext_exports = {};
__export(ChatContext_exports, {
  ChatContext: () => ChatContext,
  ChatContextProvider: () => ChatContextProvider,
  useChatContext: () => useChatContext
});
module.exports = __toCommonJS(ChatContext_exports);
var import_react = __toESM(require("react"));

// src/components/chat/Icons.tsx
var import_jsx_runtime = require("react/jsx-runtime");
var OpenIcon = /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
  "svg",
  {
    xmlns: "http://www.w3.org/2000/svg",
    viewBox: "0 0 24 24",
    fill: "currentColor",
    width: "24",
    height: "24",
    children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("g", { transform: "translate(24, 0) scale(-1, 1)", children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
      "path",
      {
        fillRule: "evenodd",
        d: "M5.337 21.718a6.707 6.707 0 01-.533-.074.75.75 0 01-.44-1.223 3.73 3.73 0 00.814-1.686c.023-.115-.022-.317-.254-.543C3.274 16.587 2.25 14.41 2.25 12c0-5.03 4.428-9 9.75-9s9.75 3.97 9.75 9c0 5.03-4.428 9-9.75 9-.833 0-1.643-.097-2.417-.279a6.721 6.721 0 01-4.246.997z",
        clipRule: "evenodd"
      }
    ) })
  }
);
var CloseIcon = /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
  "svg",
  {
    xmlns: "http://www.w3.org/2000/svg",
    fill: "none",
    viewBox: "0 0 24 24",
    strokeWidth: "1.5",
    stroke: "currentColor",
    width: "24",
    height: "24",
    children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { strokeLinecap: "round", strokeLinejoin: "round", d: "M19.5 8.25l-7.5 7.5-7.5-7.5" })
  }
);
var HeaderCloseIcon = /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
  "svg",
  {
    xmlns: "http://www.w3.org/2000/svg",
    fill: "none",
    viewBox: "0 0 24 24",
    strokeWidth: "1.5",
    stroke: "currentColor",
    width: "24",
    height: "24",
    children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { strokeLinecap: "round", strokeLinejoin: "round", d: "M6 18L18 6M6 6l12 12" })
  }
);
var SendIcon = /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
  "svg",
  {
    xmlns: "http://www.w3.org/2000/svg",
    fill: "none",
    viewBox: "0 0 24 24",
    strokeWidth: "1.5",
    stroke: "currentColor",
    width: "24",
    height: "24",
    children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { strokeLinecap: "round", strokeLinejoin: "round", d: "M12 19V5m0 0l-7 7m7-7l7 7" })
  }
);
var MicrophoneIcon = /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
  "svg",
  {
    xmlns: "http://www.w3.org/2000/svg",
    fill: "none",
    viewBox: "0 0 24 24",
    strokeWidth: "1.5",
    stroke: "currentColor",
    width: "24",
    height: "24",
    children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
      "path",
      {
        strokeLinecap: "round",
        strokeLinejoin: "round",
        d: "M12 18.75a6 6 0 006-6v-1.5m-6 7.5a6 6 0 01-6-6v-1.5m6 7.5v3.75m-3.75 0h7.5M12 15.75a3 3 0 01-3-3V4.5a3 3 0 116 0v8.25a3 3 0 01-3 3z"
      }
    )
  }
);
var StopIcon = /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
  "svg",
  {
    xmlns: "http://www.w3.org/2000/svg",
    fill: "none",
    viewBox: "0 0 24 24",
    strokeWidth: "1.5",
    stroke: "currentColor",
    width: "24",
    height: "24",
    children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
      "path",
      {
        strokeLinecap: "round",
        strokeLinejoin: "round",
        d: "M5.25 7.5A2.25 2.25 0 017.5 5.25h9a2.25 2.25 0 012.25 2.25v9a2.25 2.25 0 01-2.25 2.25h-9a2.25 2.25 0 01-2.25-2.25v-9z"
      }
    )
  }
);
var RegenerateIcon = /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
  "svg",
  {
    xmlns: "http://www.w3.org/2000/svg",
    fill: "none",
    viewBox: "0 0 24 24",
    strokeWidth: "2",
    stroke: "currentColor",
    width: "16",
    height: "16",
    style: { minWidth: "16px", minHeight: "16px" },
    children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
      "path",
      {
        strokeLinecap: "round",
        strokeLinejoin: "round",
        d: "M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0l3.181 3.183a8.25 8.25 0 0013.803-3.7M4.031 9.865a8.25 8.25 0 0113.803-3.7l3.181 3.182m0-4.991v4.99"
      }
    )
  }
);
var CopyIcon = /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
  "svg",
  {
    xmlns: "http://www.w3.org/2000/svg",
    fill: "none",
    viewBox: "0 0 24 24",
    strokeWidth: "2",
    stroke: "currentColor",
    width: "16",
    height: "16",
    style: { minWidth: "16px", minHeight: "16px" },
    children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
      "path",
      {
        strokeLinecap: "round",
        strokeLinejoin: "round",
        d: "M15.75 17.25v3.375c0 .621-.504 1.125-1.125 1.125h-9.75a1.125 1.125 0 01-1.125-1.125V7.875c0-.621.504-1.125 1.125-1.125H6.75a9.06 9.06 0 011.5.124m7.5 10.376h3.375c.621 0 1.125-.504 1.125-1.125V11.25c0-4.46-3.243-8.161-7.5-8.876a9.06 9.06 0 00-1.5-.124H9.375c-.621 0-1.125.504-1.125 1.125v3.5m7.5 10.375H9.375a1.125 1.125 0 01-1.125-1.125v-9.25m12 6.625v-1.875a3.375 3.375 0 00-3.375-3.375h-1.5a1.125 1.125 0 01-1.125-1.125v-1.5a3.375 3.375 0 00-3.375-3.375H9.75"
      }
    )
  }
);
var SpinnerIcon = /* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", { className: "copilotKitSpinner", style: { width: "24px", height: "24px" } });
var ActivityIcon = /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", { style: { display: "flex", alignItems: "center", gap: "4px" }, children: [
  /* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", { className: "copilotKitActivityDot", style: { animationDelay: "0s" } }),
  /* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", { className: "copilotKitActivityDot", style: { animationDelay: "0.2s" } }),
  /* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", { className: "copilotKitActivityDot", style: { animationDelay: "0.4s" } })
] });
var ThumbsUpIcon = /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
  "svg",
  {
    xmlns: "http://www.w3.org/2000/svg",
    fill: "none",
    viewBox: "0 0 24 24",
    strokeWidth: "2",
    stroke: "currentColor",
    width: "16",
    height: "16",
    style: { minWidth: "16px", minHeight: "16px" },
    children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
      "path",
      {
        strokeLinecap: "round",
        strokeLinejoin: "round",
        d: "M6.633 10.5c.806 0 1.533-.446 2.031-1.08a9.041 9.041 0 012.861-2.4c.723-.384 1.35-.956 1.653-1.715a4.498 4.498 0 00.322-1.672V3a.75.75 0 01.75-.75A2.25 2.25 0 0116.5 4.5c0 1.152-.26 2.243-.723 3.218-.266.558.107 1.282.725 1.282h3.126c1.026 0 1.945.694 2.054 1.715.045.422.068.85.068 1.285a11.95 11.95 0 01-2.649 7.521c-.388.482-.987.729-1.605.729H13.48c-.483 0-.964-.078-1.423-.23l-3.114-1.04a4.501 4.501 0 00-1.423-.23H5.904M14.25 9h2.25M5.904 18.75c.083.205.173.405.27.602.197.4-.078.898-.523.898h-.908c-.889 0-1.713-.518-1.972-1.368a12 12 0 01-.521-3.507c0-1.553.295-3.036.831-4.398C3.387 10.203 4.167 9.75 5 9.75h1.053c.472 0 .745.556.5.96a8.958 8.958 0 00-1.302 4.665c0 1.194.232 2.333.654 3.375z"
      }
    )
  }
);
var ThumbsDownIcon = /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
  "svg",
  {
    xmlns: "http://www.w3.org/2000/svg",
    fill: "none",
    viewBox: "0 0 24 24",
    strokeWidth: "2",
    stroke: "currentColor",
    width: "16",
    height: "16",
    style: { minWidth: "16px", minHeight: "16px" },
    children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
      "path",
      {
        strokeLinecap: "round",
        strokeLinejoin: "round",
        d: "M7.5 15h2.25m8.024-9.75c.011.05.028.1.052.148.591 1.2.924 2.55.924 3.977a8.96 8.96 0 01-.999 4.125m.023-8.25c-.076-.365.183-.75.575-.75h.908c.889 0 1.713.518 1.972 1.368.339 1.11.521 2.287.521 3.507 0 1.553-.295 3.036-.831 4.398C20.613 14.547 19.833 15 19 15h-1.053c-.472 0-.745-.556-.5-.96a8.95 8.95 0 00.303-.54m.023-8.25H16.48a4.5 4.5 0 01-1.423-.23l-3.114-1.04a4.5 4.5 0 00-1.423-.23H6.504c-.618 0-1.217.247-1.605.729A11.95 11.95 0 002.25 12c0 .434.023.863.068 1.285C2.427 14.306 3.346 15 4.372 15h3.126c.618 0 .991.724.725 1.282A7.471 7.471 0 007.5 19.5a2.25 2.25 0 002.25 2.25.75.75 0 00.75-.75v-.633c0-.573.11-1.14.322-1.672.304-.76.93-1.33 1.653-1.715a9.04 9.04 0 002.86-2.4c.498-.634 1.226-1.08 2.032-1.08h.384"
      }
    )
  }
);
var UploadIcon = /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
  "svg",
  {
    xmlns: "http://www.w3.org/2000/svg",
    fill: "none",
    viewBox: "0 0 24 24",
    strokeWidth: "1.5",
    stroke: "currentColor",
    width: "24",
    height: "24",
    children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { strokeLinecap: "round", strokeLinejoin: "round", d: "M12 4.5v15m7.5-7.5h-15" })
  }
);

// src/components/chat/ChatContext.tsx
var import_jsx_runtime2 = require("react/jsx-runtime");
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
var ChatContextProvider = ({
  // temperature,
  // instructions,
  // maxFeedback,
  labels,
  icons,
  children,
  open,
  setOpen
}) => {
  const memoizedLabels = (0, import_react.useMemo)(
    () => __spreadValues(__spreadValues({}, {
      initial: "",
      title: "CopilotKit",
      placeholder: "Type a message...",
      error: "\u274C An error occurred. Please try again.",
      stopGenerating: "Stop generating",
      regenerateResponse: "Regenerate response",
      copyToClipboard: "Copy to clipboard",
      thumbsUp: "Thumbs up",
      thumbsDown: "Thumbs down",
      copied: "Copied!"
    }), labels),
    [labels]
  );
  const memoizedIcons = (0, import_react.useMemo)(
    () => __spreadValues(__spreadValues({}, {
      openIcon: OpenIcon,
      closeIcon: CloseIcon,
      headerCloseIcon: HeaderCloseIcon,
      sendIcon: SendIcon,
      activityIcon: ActivityIcon,
      spinnerIcon: SpinnerIcon,
      stopIcon: StopIcon,
      regenerateIcon: RegenerateIcon,
      pushToTalkIcon: MicrophoneIcon,
      copyIcon: CopyIcon,
      thumbsUpIcon: ThumbsUpIcon,
      thumbsDownIcon: ThumbsDownIcon,
      uploadIcon: UploadIcon
    }), icons),
    [icons]
  );
  const context = (0, import_react.useMemo)(
    () => ({
      labels: memoizedLabels,
      icons: memoizedIcons,
      open,
      setOpen
    }),
    [memoizedLabels, memoizedIcons, open, setOpen]
  );
  return /* @__PURE__ */ (0, import_jsx_runtime2.jsx)(ChatContext.Provider, { value: context, children });
};
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  ChatContext,
  ChatContextProvider,
  useChatContext
});
//# sourceMappingURL=ChatContext.js.map