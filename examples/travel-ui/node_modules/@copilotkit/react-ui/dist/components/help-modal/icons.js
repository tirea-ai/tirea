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

// src/components/help-modal/icons.tsx
var icons_exports = {};
__export(icons_exports, {
  CloseIcon: () => CloseIcon,
  LifeBuoyIcon: () => LifeBuoyIcon,
  LoadingSpinnerIcon: () => LoadingSpinnerIcon
});
module.exports = __toCommonJS(icons_exports);
var import_jsx_runtime = require("react/jsx-runtime");
var LifeBuoyIcon = () => /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
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
    className: "icon icon-tabler icons-tabler-outline icon-tabler-lifebuoy",
    children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("g", { transform: "translate(0, -1)", children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { stroke: "none", d: "M0 0h24v24H0z", fill: "none" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { d: "M12 12m-4 0a4 4 0 1 0 8 0a4 4 0 1 0 -8 0" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { d: "M12 12m-9 0a9 9 0 1 0 18 0a9 9 0 1 0 -18 0" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { d: "M15 15l3.35 3.35" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { d: "M9 15l-3.35 3.35" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { d: "M5.65 5.65l3.35 3.35" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { d: "M18.35 5.65l-3.35 3.35" })
    ] })
  }
);
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
var LoadingSpinnerIcon = ({ color = "rgb(107 114 128)" }) => /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(
  "svg",
  {
    style: {
      animation: "copilotKitSpinAnimation 1s linear infinite",
      color
    },
    width: "24",
    height: "24",
    xmlns: "http://www.w3.org/2000/svg",
    fill: "none",
    viewBox: "0 0 24 24",
    children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
        "circle",
        {
          style: { opacity: 0.25 },
          cx: "12",
          cy: "12",
          r: "10",
          stroke: "currentColor",
          strokeWidth: "4"
        }
      ),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
        "path",
        {
          style: { opacity: 0.75 },
          fill: "currentColor",
          d: "M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
        }
      )
    ]
  }
);
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  CloseIcon,
  LifeBuoyIcon,
  LoadingSpinnerIcon
});
//# sourceMappingURL=icons.js.map