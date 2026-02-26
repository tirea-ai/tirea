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

// src/graphql/@generated/fragment-masking.ts
var fragment_masking_exports = {};
__export(fragment_masking_exports, {
  isFragmentReady: () => isFragmentReady,
  makeFragmentData: () => makeFragmentData,
  useFragment: () => useFragment
});
module.exports = __toCommonJS(fragment_masking_exports);
function useFragment(_documentNode, fragmentType) {
  return fragmentType;
}
function makeFragmentData(data, _fragment) {
  return data;
}
function isFragmentReady(queryNode, fragmentNode, data) {
  var _a, _b;
  const deferredFields = (_a = queryNode.__meta__) == null ? void 0 : _a.deferredFields;
  if (!deferredFields)
    return true;
  const fragDef = fragmentNode.definitions[0];
  const fragName = (_b = fragDef == null ? void 0 : fragDef.name) == null ? void 0 : _b.value;
  const fields = fragName && deferredFields[fragName] || [];
  return fields.length > 0 && fields.every((field) => data && field in data);
}
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  isFragmentReady,
  makeFragmentData,
  useFragment
});
//# sourceMappingURL=fragment-masking.js.map