// src/hooks/use-copy-to-clipboard.tsx
import * as React from "react";
function useCopyToClipboard({ timeout = 2e3 }) {
  const [isCopied, setIsCopied] = React.useState(false);
  const copyToClipboard = (value) => {
    var _a;
    if (typeof window === "undefined" || !((_a = navigator.clipboard) == null ? void 0 : _a.writeText)) {
      return;
    }
    if (!value) {
      return;
    }
    navigator.clipboard.writeText(value).then(() => {
      setIsCopied(true);
      setTimeout(() => {
        setIsCopied(false);
      }, timeout);
    });
  };
  return { isCopied, copyToClipboard };
}

export {
  useCopyToClipboard
};
//# sourceMappingURL=chunk-54JAUBUJ.mjs.map