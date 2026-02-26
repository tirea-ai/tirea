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

// src/hooks/use-dark-mode.ts
var use_dark_mode_exports = {};
__export(use_dark_mode_exports, {
  useDarkMode: () => useDarkMode
});
module.exports = __toCommonJS(use_dark_mode_exports);
var useDarkMode = () => {
  if (typeof window === "undefined")
    return false;
  return document.documentElement.classList.contains("dark") || document.body.classList.contains("dark") || document.documentElement.getAttribute("data-theme") === "dark" || document.body.getAttribute("data-theme") === "dark" || window.matchMedia("(prefers-color-scheme: dark)").matches;
};
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  useDarkMode
});
//# sourceMappingURL=use-dark-mode.js.map