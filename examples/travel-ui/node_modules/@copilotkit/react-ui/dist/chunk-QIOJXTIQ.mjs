// src/components/chat/Textarea.tsx
import { useState, useRef, useEffect, forwardRef, useImperativeHandle } from "react";
import { jsx } from "react/jsx-runtime";
var AutoResizingTextarea = forwardRef(
  ({
    maxRows = 1,
    placeholder,
    value,
    onChange,
    onKeyDown,
    onCompositionStart,
    onCompositionEnd,
    autoFocus
  }, ref) => {
    const internalTextareaRef = useRef(null);
    const [maxHeight, setMaxHeight] = useState(0);
    useImperativeHandle(ref, () => internalTextareaRef.current);
    useEffect(() => {
      const calculateMaxHeight = () => {
        const textarea = internalTextareaRef.current;
        if (textarea) {
          textarea.style.height = "auto";
          const singleRowHeight = textarea.scrollHeight;
          setMaxHeight(singleRowHeight * maxRows);
          if (autoFocus) {
            textarea.focus();
          }
        }
      };
      calculateMaxHeight();
    }, [maxRows]);
    useEffect(() => {
      const textarea = internalTextareaRef.current;
      if (textarea) {
        textarea.style.height = "auto";
        textarea.style.height = `${Math.min(textarea.scrollHeight, maxHeight)}px`;
      }
    }, [value, maxHeight]);
    return /* @__PURE__ */ jsx(
      "textarea",
      {
        ref: internalTextareaRef,
        value,
        onChange,
        onKeyDown,
        onCompositionStart,
        onCompositionEnd,
        placeholder,
        style: {
          overflow: "auto",
          resize: "none",
          maxHeight: `${maxHeight}px`
        },
        rows: 1
      }
    );
  }
);
var Textarea_default = AutoResizingTextarea;

export {
  Textarea_default
};
//# sourceMappingURL=chunk-QIOJXTIQ.mjs.map