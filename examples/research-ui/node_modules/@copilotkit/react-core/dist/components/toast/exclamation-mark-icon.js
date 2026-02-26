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

// src/components/toast/exclamation-mark-icon.tsx
var exclamation_mark_icon_exports = {};
__export(exclamation_mark_icon_exports, {
  ExclamationMarkIcon: () => ExclamationMarkIcon
});
module.exports = __toCommonJS(exclamation_mark_icon_exports);
var import_jsx_runtime = require("react/jsx-runtime");
var ExclamationMarkIcon = ({
  className,
  style
}) => /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(
  "svg",
  {
    xmlns: "http://www.w3.org/2000/svg",
    width: "24",
    height: "24",
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: "2",
    strokeLinecap: "round",
    strokeLinejoin: "round",
    className: `lucide lucide-circle-alert ${className ? className : ""}`,
    style,
    children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("circle", { cx: "12", cy: "12", r: "10" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("line", { x1: "12", x2: "12", y1: "8", y2: "12" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("line", { x1: "12", x2: "12.01", y1: "16", y2: "16" })
    ]
  }
);
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  ExclamationMarkIcon
});
//# sourceMappingURL=exclamation-mark-icon.js.map