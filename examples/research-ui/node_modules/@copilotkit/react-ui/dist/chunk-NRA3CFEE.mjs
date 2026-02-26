import {
  CloseIcon
} from "./chunk-BH6PCAAL.mjs";

// src/components/help-modal/modal.tsx
import { useState, useRef, useEffect } from "react";
import { jsx, jsxs } from "react/jsx-runtime";
function CopilotKitHelpModal() {
  const [showHelpModal, setShowHelpModal] = useState(false);
  const buttonRef = useRef(null);
  const popoverRef = useRef(null);
  useEffect(() => {
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
  const HelpButton = () => /* @__PURE__ */ jsx(
    "button",
    {
      ref: buttonRef,
      onClick: () => setShowHelpModal(!showHelpModal),
      className: "copilotKitDebugMenuTriggerButton relative",
      "aria-label": "Open Help",
      children: "Help"
    }
  );
  return /* @__PURE__ */ jsxs("div", { className: "relative", children: [
    /* @__PURE__ */ jsx(HelpButton, {}),
    showHelpModal && /* @__PURE__ */ jsx(
      "div",
      {
        ref: popoverRef,
        className: "absolute mt-2 z-50",
        style: {
          top: "100%",
          right: "-120px",
          width: "380px"
        },
        children: /* @__PURE__ */ jsxs("div", { className: "copilotKitHelpModal rounded-lg shadow-xl w-full p-4 flex-col relative", children: [
          /* @__PURE__ */ jsx(
            "button",
            {
              className: "copilotKitHelpModalCloseButton absolute text-gray-400 hover:text-gray-600 focus:outline-none",
              style: { top: "10px", right: "10px" },
              onClick: () => setShowHelpModal(false),
              "aria-label": "Close",
              children: /* @__PURE__ */ jsx(CloseIcon, {})
            }
          ),
          /* @__PURE__ */ jsx("div", { className: "w-full flex mb-6 justify-center", children: /* @__PURE__ */ jsx("h2", { className: "text-2xl font-bold", children: "Help Options" }) }),
          /* @__PURE__ */ jsxs("div", { className: "space-y-4 mb-4", children: [
            /* @__PURE__ */ jsx("div", { className: "copilotKitHelpItemButton", children: /* @__PURE__ */ jsx(
              "a",
              {
                href: "https://docs.copilotkit.ai/coagents/troubleshooting/common-issues",
                target: "_blank",
                rel: "noopener noreferrer",
                children: "Visit the Troubleshooting and FAQ section in the docs"
              }
            ) }),
            /* @__PURE__ */ jsx("div", { className: "copilotKitHelpItemButton", children: /* @__PURE__ */ jsx(
              "a",
              {
                href: "https://go.copilotkit.ai/dev-console-support-discord",
                target: "_blank",
                rel: "noopener noreferrer",
                children: "Go to Discord Support Channel (Community Support)"
              }
            ) }),
            /* @__PURE__ */ jsx("div", { className: "copilotKitHelpItemButton", children: /* @__PURE__ */ jsx(
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

export {
  CopilotKitHelpModal
};
//# sourceMappingURL=chunk-NRA3CFEE.mjs.map