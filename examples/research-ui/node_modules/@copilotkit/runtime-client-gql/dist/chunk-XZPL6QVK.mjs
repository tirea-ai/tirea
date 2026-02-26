// src/graphql/@generated/fragment-masking.ts
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

export {
  useFragment,
  makeFragmentData,
  isFragmentReady
};
//# sourceMappingURL=chunk-XZPL6QVK.mjs.map