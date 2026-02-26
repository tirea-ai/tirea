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

// src/components/chat/Window.tsx
var Window_exports = {};
__export(Window_exports, {
  Window: () => Window
});
module.exports = __toCommonJS(Window_exports);
var import_react2 = __toESM(require("react"));

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

// src/components/chat/Window.tsx
var import_react_core = require("@copilotkit/react-core");
var import_shared = require("@copilotkit/shared");
var import_jsx_runtime2 = require("react/jsx-runtime");
var Window = ({
  children,
  clickOutsideToClose,
  shortcut,
  hitEscapeToClose
}) => {
  const windowRef = import_react2.default.useRef(null);
  const context = (0, import_react_core.useCopilotContext)();
  const { open, setOpen } = useChatContext();
  const handleClickOutside = (0, import_react2.useCallback)(
    (event) => {
      var _a;
      if (!clickOutsideToClose) {
        return;
      }
      const parentElement = (_a = windowRef.current) == null ? void 0 : _a.parentElement;
      let className = "";
      if (event.target instanceof HTMLElement) {
        className = event.target.className;
      }
      if (open && parentElement && !parentElement.contains(event.target) && // prevent closing the window when clicking on the debug menu
      !className.includes("copilotKitDebugMenu")) {
        setOpen(false);
      }
    },
    [clickOutsideToClose, open, setOpen]
  );
  const handleKeyDown = (0, import_react2.useCallback)(
    (event) => {
      var _a;
      const target = event.target;
      const isInput = target.tagName === "INPUT" || target.tagName === "SELECT" || target.tagName === "TEXTAREA" || target.isContentEditable;
      const isDescendantOfWrapper = (_a = windowRef.current) == null ? void 0 : _a.contains(target);
      if (open && event.key === "Escape" && (!isInput || isDescendantOfWrapper) && hitEscapeToClose) {
        setOpen(false);
      } else if (event.key === shortcut && ((0, import_shared.isMacOS)() && event.metaKey || !(0, import_shared.isMacOS)() && event.ctrlKey) && (!isInput || isDescendantOfWrapper)) {
        setOpen(!open);
      }
    },
    [hitEscapeToClose, shortcut, open, setOpen]
  );
  const adjustForMobile = (0, import_react2.useCallback)(() => {
    const copilotKitWindow = windowRef.current;
    const vv = window.visualViewport;
    if (!copilotKitWindow || !vv) {
      return;
    }
    if (window.innerWidth < 640 && open) {
      copilotKitWindow.style.height = `${vv.height}px`;
      copilotKitWindow.style.left = `${vv.offsetLeft}px`;
      copilotKitWindow.style.top = `${vv.offsetTop}px`;
      document.body.style.position = "fixed";
      document.body.style.width = "100%";
      document.body.style.height = `${window.innerHeight}px`;
      document.body.style.overflow = "hidden";
      document.body.style.touchAction = "none";
      document.body.addEventListener("touchmove", preventScroll, {
        passive: false
      });
    } else {
      copilotKitWindow.style.height = "";
      copilotKitWindow.style.left = "";
      copilotKitWindow.style.top = "";
      document.body.style.position = "";
      document.body.style.height = "";
      document.body.style.width = "";
      document.body.style.overflow = "";
      document.body.style.top = "";
      document.body.style.touchAction = "";
      document.body.removeEventListener("touchmove", preventScroll);
    }
  }, [open]);
  (0, import_react2.useEffect)(() => {
    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleKeyDown);
    if (window.visualViewport) {
      window.visualViewport.addEventListener("resize", adjustForMobile);
      adjustForMobile();
    }
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("keydown", handleKeyDown);
      if (window.visualViewport) {
        window.visualViewport.removeEventListener("resize", adjustForMobile);
      }
    };
  }, [adjustForMobile, handleClickOutside, handleKeyDown]);
  return /* @__PURE__ */ (0, import_jsx_runtime2.jsx)("div", { className: `copilotKitWindow ${open ? " open" : ""}`, ref: windowRef, children });
};
var preventScroll = (event) => {
  let targetElement = event.target;
  const hasParentWithClass = (element, className) => {
    while (element && element !== document.body) {
      if (element.classList.contains(className)) {
        return true;
      }
      element = element.parentElement;
    }
    return false;
  };
  if (!hasParentWithClass(targetElement, "copilotKitMessages")) {
    event.preventDefault();
  }
};
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  Window
});
//# sourceMappingURL=Window.js.map