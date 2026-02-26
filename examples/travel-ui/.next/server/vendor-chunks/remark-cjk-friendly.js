"use strict";
/*
 * ATTENTION: An "eval-source-map" devtool has been used.
 * This devtool is neither made for production nor for readable output files.
 * It uses "eval()" calls to create a separate source file with attached SourceMaps in the browser devtools.
 * If you are trying to read the output file, select a different devtool (https://webpack.js.org/configuration/devtool/)
 * or disable the default devtool with "devtool: false".
 * If you are looking for production-ready output files, see mode: "production" (https://webpack.js.org/configuration/mode/).
 */
exports.id = "vendor-chunks/remark-cjk-friendly";
exports.ids = ["vendor-chunks/remark-cjk-friendly"];
exports.modules = {

/***/ "(ssr)/./node_modules/remark-cjk-friendly/dist/index.js":
/*!********************************************************!*\
  !*** ./node_modules/remark-cjk-friendly/dist/index.js ***!
  \********************************************************/
/***/ ((__unused_webpack___webpack_module__, __webpack_exports__, __webpack_require__) => {

eval("__webpack_require__.r(__webpack_exports__);\n/* harmony export */ __webpack_require__.d(__webpack_exports__, {\n/* harmony export */   \"default\": () => (/* binding */ remarkCjkFriendly)\n/* harmony export */ });\n/* harmony import */ var micromark_extension_cjk_friendly__WEBPACK_IMPORTED_MODULE_0__ = __webpack_require__(/*! micromark-extension-cjk-friendly */ \"(ssr)/./node_modules/micromark-extension-cjk-friendly/dist/index.js\");\n// src/index.ts\n\nfunction remarkCjkFriendly() {\n  const data = this.data();\n  const micromarkExtensions = (\n    // biome-ignore lint/suspicious/noAssignInExpressions: base plugin (remark-gfm) already does this\n    data.micromarkExtensions || (data.micromarkExtensions = [])\n  );\n  micromarkExtensions.push((0,micromark_extension_cjk_friendly__WEBPACK_IMPORTED_MODULE_0__.cjkFriendlyExtension)());\n}\n\n//# sourceURL=[module]\n//# sourceMappingURL=data:application/json;charset=utf-8;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiKHNzcikvLi9ub2RlX21vZHVsZXMvcmVtYXJrLWNqay1mcmllbmRseS9kaXN0L2luZGV4LmpzIiwibWFwcGluZ3MiOiI7Ozs7O0FBQUE7QUFDd0U7QUFDeEU7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0EsMkJBQTJCLHNGQUFvQjtBQUMvQztBQUdFIiwic291cmNlcyI6WyIvaG9tZS9jaGFpemhlbmh1YS9Db2Rlcy91bmNhcnZlL2UyZS9leGFtcGxlcy90cmF2ZWwtdWkvbm9kZV9tb2R1bGVzL3JlbWFyay1jamstZnJpZW5kbHkvZGlzdC9pbmRleC5qcyJdLCJzb3VyY2VzQ29udGVudCI6WyIvLyBzcmMvaW5kZXgudHNcbmltcG9ydCB7IGNqa0ZyaWVuZGx5RXh0ZW5zaW9uIH0gZnJvbSBcIm1pY3JvbWFyay1leHRlbnNpb24tY2prLWZyaWVuZGx5XCI7XG5mdW5jdGlvbiByZW1hcmtDamtGcmllbmRseSgpIHtcbiAgY29uc3QgZGF0YSA9IHRoaXMuZGF0YSgpO1xuICBjb25zdCBtaWNyb21hcmtFeHRlbnNpb25zID0gKFxuICAgIC8vIGJpb21lLWlnbm9yZSBsaW50L3N1c3BpY2lvdXMvbm9Bc3NpZ25JbkV4cHJlc3Npb25zOiBiYXNlIHBsdWdpbiAocmVtYXJrLWdmbSkgYWxyZWFkeSBkb2VzIHRoaXNcbiAgICBkYXRhLm1pY3JvbWFya0V4dGVuc2lvbnMgfHwgKGRhdGEubWljcm9tYXJrRXh0ZW5zaW9ucyA9IFtdKVxuICApO1xuICBtaWNyb21hcmtFeHRlbnNpb25zLnB1c2goY2prRnJpZW5kbHlFeHRlbnNpb24oKSk7XG59XG5leHBvcnQge1xuICByZW1hcmtDamtGcmllbmRseSBhcyBkZWZhdWx0XG59O1xuIl0sIm5hbWVzIjpbXSwiaWdub3JlTGlzdCI6WzBdLCJzb3VyY2VSb290IjoiIn0=\n//# sourceURL=webpack-internal:///(ssr)/./node_modules/remark-cjk-friendly/dist/index.js\n");

/***/ })

};
;