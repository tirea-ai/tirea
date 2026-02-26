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

// src/components/help-modal/index.tsx
var help_modal_exports = {};
__export(help_modal_exports, {
  CopilotKitHelpModal: () => CopilotKitHelpModal
});
module.exports = __toCommonJS(help_modal_exports);

// src/components/help-modal/modal.tsx
var import_react = require("react");

// src/components/help-modal/icons.tsx
var import_jsx_runtime = require("react/jsx-runtime");
var CloseIcon = () => /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
  "svg",
  {
    xmlns: "http://www.w3.org/2000/svg",
    fill: "none",
    viewBox: "0 0 24 24",
    strokeWidth: "1.5",
    stroke: "currentColor",
    width: "20",
    height: "20",
    children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { strokeLinecap: "round", strokeLinejoin: "round", d: "M6 18L18 6M6 6l12 12" })
  }
);

// src/components/help-modal/modal.tsx
var import_jsx_runtime2 = require("react/jsx-runtime");
function CopilotKitHelpModal() {
  const [showHelpModal, setShowHelpModal] = (0, import_react.useState)(false);
  const buttonRef = (0, import_react.useRef)(null);
  const popoverRef = (0, import_react.useRef)(null);
  (0, import_react.useEffect)(() => {
    const handleClickOutside = (event) => {
      if (popoverRef.current && !popoverRef.current.contains(event.target) && buttonRef.current && !buttonRef.current.contains(event.target)) {
        setShowHelpModal(false);
      }
    };
    if (showHelpModal) {
      document.addEventListener("mousedown", handleClickOutside);
    }
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, [showHelpModal]);
  const HelpButton = () => /* @__PURE__ */ (0, import_jsx_runtime2.jsx)(
    "button",
    {
      ref: buttonRef,
      onClick: () => setShowHelpModal(!showHelpModal),
      className: "copilotKitDebugMenuTriggerButton relative",
      "aria-label": "Open Help",
      children: "Help"
    }
  );
  return /* @__PURE__ */ (0, import_jsx_runtime2.jsxs)("div", { className: "relative", children: [
    /* @__PURE__ */ (0, import_jsx_runtime2.jsx)(HelpButton, {}),
    showHelpModal && /* @__PURE__ */ (0, import_jsx_runtime2.jsx)(
      "div",
      {
        ref: popoverRef,
        className: "absolute mt-2 z-50",
        style: {
          top: "100%",
          right: "-120px",
          width: "380px"
        },
        children: /* @__PURE__ */ (0, import_jsx_runtime2.jsxs)("div", { className: "copilotKitHelpModal rounded-lg shadow-xl w-full p-4 flex-col relative", children: [
          /* @__PURE__ */ (0, import_jsx_runtime2.jsx)(
            "button",
            {
              className: "copilotKitHelpModalCloseButton absolute text-gray-400 hover:text-gray-600 focus:outline-none",
              style: { top: "10px", right: "10px" },
              onClick: () => setShowHelpModal(false),
              "aria-label": "Close",
              children: /* @__PURE__ */ (0, import_jsx_runtime2.jsx)(CloseIcon, {})
            }
          ),
          /* @__PURE__ */ (0, import_jsx_runtime2.jsx)("div", { className: "w-full flex mb-6 justify-center", children: /* @__PURE__ */ (0, import_jsx_runtime2.jsx)("h2", { className: "text-2xl font-bold", children: "Help Options" }) }),
          /* @__PURE__ */ (0, import_jsx_runtime2.jsxs)("div", { className: "space-y-4 mb-4", children: [
            /* @__PURE__ */ (0, import_jsx_runtime2.jsx)("div", { className: "copilotKitHelpItemButton", children: /* @__PURE__ */ (0, import_jsx_runtime2.jsx)(
              "a",
              {
                href: "https://docs.copilotkit.ai/coagents/troubleshooting/common-issues",
                target: "_blank",
                rel: "noopener noreferrer",
                children: "Visit the Troubleshooting and FAQ section in the docs"
              }
            ) }),
            /* @__PURE__ */ (0, import_jsx_runtime2.jsx)("div", { className: "copilotKitHelpItemButton", children: /* @__PURE__ */ (0, import_jsx_runtime2.jsx)(
              "a",
              {
                href: "https://go.copilotkit.ai/dev-console-support-discord",
                target: "_blank",
                rel: "noopener noreferrer",
                children: "Go to Discord Support Channel (Community Support)"
              }
            ) }),
            /* @__PURE__ */ (0, import_jsx_runtime2.jsx)("div", { className: "copilotKitHelpItemButton", children: /* @__PURE__ */ (0, import_jsx_runtime2.jsx)(
              "a",
              {
                href: "https://go.copilotkit.ai/dev-console-support-slack",
                target: "_blank",
                rel: "noopener noreferrer",
                children: "Apply for Priority Direct Slack Support"
              }
            ) })
          ] })
        ] })
      }
    )
  ] });
}
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  CopilotKitHelpModal
});
//# sourceMappingURL=index.js.map