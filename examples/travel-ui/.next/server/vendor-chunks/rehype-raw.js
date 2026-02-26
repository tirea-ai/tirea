"use strict";
/*
 * ATTENTION: An "eval-source-map" devtool has been used.
 * This devtool is neither made for production nor for readable output files.
 * It uses "eval()" calls to create a separate source file with attached SourceMaps in the browser devtools.
 * If you are trying to read the output file, select a different devtool (https://webpack.js.org/configuration/devtool/)
 * or disable the default devtool with "devtool: false".
 * If you are looking for production-ready output files, see mode: "production" (https://webpack.js.org/configuration/mode/).
 */
exports.id = "vendor-chunks/rehype-raw";
exports.ids = ["vendor-chunks/rehype-raw"];
exports.modules = {

/***/ "(ssr)/./node_modules/rehype-raw/lib/index.js":
/*!**********************************************!*\
  !*** ./node_modules/rehype-raw/lib/index.js ***!
  \**********************************************/
/***/ ((__unused_webpack___webpack_module__, __webpack_exports__, __webpack_require__) => {

eval("__webpack_require__.r(__webpack_exports__);\n/* harmony export */ __webpack_require__.d(__webpack_exports__, {\n/* harmony export */   \"default\": () => (/* binding */ rehypeRaw)\n/* harmony export */ });\n/* harmony import */ var hast_util_raw__WEBPACK_IMPORTED_MODULE_0__ = __webpack_require__(/*! hast-util-raw */ \"(ssr)/./node_modules/hast-util-raw/lib/index.js\");\n/**\n * @typedef {import('hast').Root} Root\n * @typedef {import('hast-util-raw').Options} RawOptions\n * @typedef {import('vfile').VFile} VFile\n */\n\n/**\n * @typedef {Omit<RawOptions, 'file'>} Options\n *   Configuration.\n */\n\n\n\n/**\n * Parse the tree (and raw nodes) again, keeping positional info okay.\n *\n * @param {Options | null | undefined}  [options]\n *   Configuration (optional).\n * @returns\n *   Transform.\n */\nfunction rehypeRaw(options) {\n  /**\n   * @param {Root} tree\n   *   Tree.\n   * @param {VFile} file\n   *   File.\n   * @returns {Root}\n   *   New tree.\n   */\n  return function (tree, file) {\n    // Assume root in -> root out.\n    const result = /** @type {Root} */ ((0,hast_util_raw__WEBPACK_IMPORTED_MODULE_0__.raw)(tree, {...options, file}))\n    return result\n  }\n}\n//# sourceURL=[module]\n//# sourceMappingURL=data:application/json;charset=utf-8;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiKHNzcikvLi9ub2RlX21vZHVsZXMvcmVoeXBlLXJhdy9saWIvaW5kZXguanMiLCJtYXBwaW5ncyI6Ijs7Ozs7QUFBQTtBQUNBLGFBQWEscUJBQXFCO0FBQ2xDLGFBQWEsaUNBQWlDO0FBQzlDLGFBQWEsdUJBQXVCO0FBQ3BDOztBQUVBO0FBQ0EsYUFBYSwwQkFBMEI7QUFDdkM7QUFDQTs7QUFFaUM7O0FBRWpDO0FBQ0E7QUFDQTtBQUNBLFdBQVcsNkJBQTZCO0FBQ3hDO0FBQ0E7QUFDQTtBQUNBO0FBQ2U7QUFDZjtBQUNBLGFBQWEsTUFBTTtBQUNuQjtBQUNBLGFBQWEsT0FBTztBQUNwQjtBQUNBLGVBQWU7QUFDZjtBQUNBO0FBQ0E7QUFDQTtBQUNBLDhCQUE4QixNQUFNLElBQUksa0RBQUcsUUFBUSxpQkFBaUI7QUFDcEU7QUFDQTtBQUNBIiwic291cmNlcyI6WyIvaG9tZS9jaGFpemhlbmh1YS9Db2Rlcy91bmNhcnZlL2UyZS9leGFtcGxlcy90cmF2ZWwtdWkvbm9kZV9tb2R1bGVzL3JlaHlwZS1yYXcvbGliL2luZGV4LmpzIl0sInNvdXJjZXNDb250ZW50IjpbIi8qKlxuICogQHR5cGVkZWYge2ltcG9ydCgnaGFzdCcpLlJvb3R9IFJvb3RcbiAqIEB0eXBlZGVmIHtpbXBvcnQoJ2hhc3QtdXRpbC1yYXcnKS5PcHRpb25zfSBSYXdPcHRpb25zXG4gKiBAdHlwZWRlZiB7aW1wb3J0KCd2ZmlsZScpLlZGaWxlfSBWRmlsZVxuICovXG5cbi8qKlxuICogQHR5cGVkZWYge09taXQ8UmF3T3B0aW9ucywgJ2ZpbGUnPn0gT3B0aW9uc1xuICogICBDb25maWd1cmF0aW9uLlxuICovXG5cbmltcG9ydCB7cmF3fSBmcm9tICdoYXN0LXV0aWwtcmF3J1xuXG4vKipcbiAqIFBhcnNlIHRoZSB0cmVlIChhbmQgcmF3IG5vZGVzKSBhZ2Fpbiwga2VlcGluZyBwb3NpdGlvbmFsIGluZm8gb2theS5cbiAqXG4gKiBAcGFyYW0ge09wdGlvbnMgfCBudWxsIHwgdW5kZWZpbmVkfSAgW29wdGlvbnNdXG4gKiAgIENvbmZpZ3VyYXRpb24gKG9wdGlvbmFsKS5cbiAqIEByZXR1cm5zXG4gKiAgIFRyYW5zZm9ybS5cbiAqL1xuZXhwb3J0IGRlZmF1bHQgZnVuY3Rpb24gcmVoeXBlUmF3KG9wdGlvbnMpIHtcbiAgLyoqXG4gICAqIEBwYXJhbSB7Um9vdH0gdHJlZVxuICAgKiAgIFRyZWUuXG4gICAqIEBwYXJhbSB7VkZpbGV9IGZpbGVcbiAgICogICBGaWxlLlxuICAgKiBAcmV0dXJucyB7Um9vdH1cbiAgICogICBOZXcgdHJlZS5cbiAgICovXG4gIHJldHVybiBmdW5jdGlvbiAodHJlZSwgZmlsZSkge1xuICAgIC8vIEFzc3VtZSByb290IGluIC0+IHJvb3Qgb3V0LlxuICAgIGNvbnN0IHJlc3VsdCA9IC8qKiBAdHlwZSB7Um9vdH0gKi8gKHJhdyh0cmVlLCB7Li4ub3B0aW9ucywgZmlsZX0pKVxuICAgIHJldHVybiByZXN1bHRcbiAgfVxufVxuIl0sIm5hbWVzIjpbXSwiaWdub3JlTGlzdCI6WzBdLCJzb3VyY2VSb290IjoiIn0=\n//# sourceURL=webpack-internal:///(ssr)/./node_modules/rehype-raw/lib/index.js\n");

/***/ })

};
;