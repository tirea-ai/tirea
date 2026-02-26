import {
  useChatContext
} from "./chunk-IEMQ2SQW.mjs";

// src/components/chat/Button.tsx
import { jsx, jsxs } from "react/jsx-runtime";
var Button = ({}) => {
  const { open, setOpen, icons } = useChatContext();
  return /* @__PURE__ */ jsx("div", { onClick: () => setOpen(!open), children: /* @__PURE__ */ jsxs(
    "button",
    {
      className: `copilotKitButton ${open ? "open" : ""}`,
      "aria-label": open ? "Close Chat" : "Open Chat",
      children: [
        /* @__PURE__ */ jsx("div", { className: "copilotKitButtonIcon copilotKitButtonIconOpen", children: icons.openIcon }),
        /* @__PURE__ */ jsx("div", { className: "copilotKitButtonIcon copilotKitButtonIconClose", children: icons.closeIcon })
      ]
    }
  ) });
};

export {
  Button
};
//# sourceMappingURL=chunk-UFN2VWSR.mjs.map