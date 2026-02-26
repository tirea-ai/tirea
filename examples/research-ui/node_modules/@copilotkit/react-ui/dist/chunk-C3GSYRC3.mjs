import {
  useChatContext
} from "./chunk-IEMQ2SQW.mjs";

// src/components/chat/Window.tsx
import React, { useCallback, useEffect } from "react";
import { useCopilotContext } from "@copilotkit/react-core";
import { isMacOS } from "@copilotkit/shared";
import { jsx } from "react/jsx-runtime";
var Window = ({
  children,
  clickOutsideToClose,
  shortcut,
  hitEscapeToClose
}) => {
  const windowRef = React.useRef(null);
  const context = useCopilotContext();
  const { open, setOpen } = useChatContext();
  const handleClickOutside = useCallback(
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
  const handleKeyDown = useCallback(
    (event) => {
      var _a;
      const target = event.target;
      const isInput = target.tagName === "INPUT" || target.tagName === "SELECT" || target.tagName === "TEXTAREA" || target.isContentEditable;
      const isDescendantOfWrapper = (_a = windowRef.current) == null ? void 0 : _a.contains(target);
      if (open && event.key === "Escape" && (!isInput || isDescendantOfWrapper) && hitEscapeToClose) {
        setOpen(false);
      } else if (event.key === shortcut && (isMacOS() && event.metaKey || !isMacOS() && event.ctrlKey) && (!isInput || isDescendantOfWrapper)) {
        setOpen(!open);
      }
    },
    [hitEscapeToClose, shortcut, open, setOpen]
  );
  const adjustForMobile = useCallback(() => {
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
  useEffect(() => {
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
  return /* @__PURE__ */ jsx("div", { className: `copilotKitWindow ${open ? " open" : ""}`, ref: windowRef, children });
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

export {
  Window
};
//# sourceMappingURL=chunk-C3GSYRC3.mjs.map