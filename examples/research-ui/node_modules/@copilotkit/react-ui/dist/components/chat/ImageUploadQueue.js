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

// src/components/chat/ImageUploadQueue.tsx
var ImageUploadQueue_exports = {};
__export(ImageUploadQueue_exports, {
  ImageUploadQueue: () => ImageUploadQueue
});
module.exports = __toCommonJS(ImageUploadQueue_exports);
var import_jsx_runtime = require("react/jsx-runtime");
var ImageUploadQueue = ({
  images,
  onRemoveImage,
  className = ""
}) => {
  if (images.length === 0)
    return null;
  return /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
    "div",
    {
      className: `copilotKitImageUploadQueue ${className}`,
      style: {
        display: "flex",
        flexWrap: "wrap",
        gap: "8px",
        margin: "8px",
        padding: "8px"
      },
      children: images.map((image, index) => /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(
        "div",
        {
          className: "copilotKitImageUploadQueueItem",
          style: {
            position: "relative",
            display: "inline-block",
            width: "60px",
            height: "60px",
            borderRadius: "4px",
            overflow: "hidden"
          },
          children: [
            /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
              "img",
              {
                src: `data:${image.contentType};base64,${image.bytes}`,
                alt: `Selected image ${index + 1}`,
                style: {
                  width: "100%",
                  height: "100%",
                  objectFit: "cover"
                }
              }
            ),
            /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
              "button",
              {
                onClick: () => onRemoveImage(index),
                className: "copilotKitImageUploadQueueRemoveButton",
                style: {
                  position: "absolute",
                  top: "2px",
                  right: "2px",
                  background: "rgba(0,0,0,0.6)",
                  color: "white",
                  border: "none",
                  borderRadius: "50%",
                  width: "18px",
                  height: "18px",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  cursor: "pointer",
                  fontSize: "10px",
                  padding: 0
                },
                children: "\u2715"
              }
            )
          ]
        },
        index
      ))
    }
  );
};
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  ImageUploadQueue
});
//# sourceMappingURL=ImageUploadQueue.js.map