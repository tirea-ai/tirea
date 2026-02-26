/*
 * ATTENTION: An "eval-source-map" devtool has been used.
 * This devtool is neither made for production nor for readable output files.
 * It uses "eval()" calls to create a separate source file with attached SourceMaps in the browser devtools.
 * If you are trying to read the output file, select a different devtool (https://webpack.js.org/configuration/devtool/)
 * or disable the default devtool with "devtool: false".
 * If you are looking for production-ready output files, see mode: "production" (https://webpack.js.org/configuration/mode/).
 */
exports.id = "vendor-chunks/style-to-object";
exports.ids = ["vendor-chunks/style-to-object"];
exports.modules = {

/***/ "(ssr)/./node_modules/style-to-object/index.js":
/*!***********************************************!*\
  !*** ./node_modules/style-to-object/index.js ***!
  \***********************************************/
/***/ ((module, __unused_webpack_exports, __webpack_require__) => {

eval("var parse = __webpack_require__(/*! inline-style-parser */ \"(ssr)/./node_modules/inline-style-parser/index.js\");\n\n/**\n * Parses inline style to object.\n *\n * @example\n * // returns { 'line-height': '42' }\n * StyleToObject('line-height: 42;');\n *\n * @param  {String}      style      - The inline style.\n * @param  {Function}    [iterator] - The iterator function.\n * @return {null|Object}\n */\nfunction StyleToObject(style, iterator) {\n  var output = null;\n  if (!style || typeof style !== 'string') {\n    return output;\n  }\n\n  var declaration;\n  var declarations = parse(style);\n  var hasIterator = typeof iterator === 'function';\n  var property;\n  var value;\n\n  for (var i = 0, len = declarations.length; i < len; i++) {\n    declaration = declarations[i];\n    property = declaration.property;\n    value = declaration.value;\n\n    if (hasIterator) {\n      iterator(property, value, declaration);\n    } else if (value) {\n      output || (output = {});\n      output[property] = value;\n    }\n  }\n\n  return output;\n}\n\nmodule.exports = StyleToObject;\nmodule.exports[\"default\"] = StyleToObject; // ESM support\n//# sourceURL=[module]\n//# sourceMappingURL=data:application/json;charset=utf-8;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiKHNzcikvLi9ub2RlX21vZHVsZXMvc3R5bGUtdG8tb2JqZWN0L2luZGV4LmpzIiwibWFwcGluZ3MiOiJBQUFBLFlBQVksbUJBQU8sQ0FBQyw4RUFBcUI7O0FBRXpDO0FBQ0E7QUFDQTtBQUNBO0FBQ0EsZ0JBQWdCO0FBQ2hCLGtDQUFrQztBQUNsQztBQUNBLFlBQVksYUFBYTtBQUN6QixZQUFZLGFBQWE7QUFDekIsWUFBWTtBQUNaO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTs7QUFFQTtBQUNBO0FBQ0E7QUFDQTtBQUNBOztBQUVBLDZDQUE2QyxTQUFTO0FBQ3REO0FBQ0E7QUFDQTs7QUFFQTtBQUNBO0FBQ0EsTUFBTTtBQUNOLDRCQUE0QjtBQUM1QjtBQUNBO0FBQ0E7O0FBRUE7QUFDQTs7QUFFQTtBQUNBLHlCQUFzQixrQkFBa0IiLCJzb3VyY2VzIjpbIi9ob21lL2NoYWl6aGVuaHVhL0NvZGVzL3VuY2FydmUvZTJlL2V4YW1wbGVzL3RyYXZlbC11aS9ub2RlX21vZHVsZXMvc3R5bGUtdG8tb2JqZWN0L2luZGV4LmpzIl0sInNvdXJjZXNDb250ZW50IjpbInZhciBwYXJzZSA9IHJlcXVpcmUoJ2lubGluZS1zdHlsZS1wYXJzZXInKTtcblxuLyoqXG4gKiBQYXJzZXMgaW5saW5lIHN0eWxlIHRvIG9iamVjdC5cbiAqXG4gKiBAZXhhbXBsZVxuICogLy8gcmV0dXJucyB7ICdsaW5lLWhlaWdodCc6ICc0MicgfVxuICogU3R5bGVUb09iamVjdCgnbGluZS1oZWlnaHQ6IDQyOycpO1xuICpcbiAqIEBwYXJhbSAge1N0cmluZ30gICAgICBzdHlsZSAgICAgIC0gVGhlIGlubGluZSBzdHlsZS5cbiAqIEBwYXJhbSAge0Z1bmN0aW9ufSAgICBbaXRlcmF0b3JdIC0gVGhlIGl0ZXJhdG9yIGZ1bmN0aW9uLlxuICogQHJldHVybiB7bnVsbHxPYmplY3R9XG4gKi9cbmZ1bmN0aW9uIFN0eWxlVG9PYmplY3Qoc3R5bGUsIGl0ZXJhdG9yKSB7XG4gIHZhciBvdXRwdXQgPSBudWxsO1xuICBpZiAoIXN0eWxlIHx8IHR5cGVvZiBzdHlsZSAhPT0gJ3N0cmluZycpIHtcbiAgICByZXR1cm4gb3V0cHV0O1xuICB9XG5cbiAgdmFyIGRlY2xhcmF0aW9uO1xuICB2YXIgZGVjbGFyYXRpb25zID0gcGFyc2Uoc3R5bGUpO1xuICB2YXIgaGFzSXRlcmF0b3IgPSB0eXBlb2YgaXRlcmF0b3IgPT09ICdmdW5jdGlvbic7XG4gIHZhciBwcm9wZXJ0eTtcbiAgdmFyIHZhbHVlO1xuXG4gIGZvciAodmFyIGkgPSAwLCBsZW4gPSBkZWNsYXJhdGlvbnMubGVuZ3RoOyBpIDwgbGVuOyBpKyspIHtcbiAgICBkZWNsYXJhdGlvbiA9IGRlY2xhcmF0aW9uc1tpXTtcbiAgICBwcm9wZXJ0eSA9IGRlY2xhcmF0aW9uLnByb3BlcnR5O1xuICAgIHZhbHVlID0gZGVjbGFyYXRpb24udmFsdWU7XG5cbiAgICBpZiAoaGFzSXRlcmF0b3IpIHtcbiAgICAgIGl0ZXJhdG9yKHByb3BlcnR5LCB2YWx1ZSwgZGVjbGFyYXRpb24pO1xuICAgIH0gZWxzZSBpZiAodmFsdWUpIHtcbiAgICAgIG91dHB1dCB8fCAob3V0cHV0ID0ge30pO1xuICAgICAgb3V0cHV0W3Byb3BlcnR5XSA9IHZhbHVlO1xuICAgIH1cbiAgfVxuXG4gIHJldHVybiBvdXRwdXQ7XG59XG5cbm1vZHVsZS5leHBvcnRzID0gU3R5bGVUb09iamVjdDtcbm1vZHVsZS5leHBvcnRzLmRlZmF1bHQgPSBTdHlsZVRvT2JqZWN0OyAvLyBFU00gc3VwcG9ydFxuIl0sIm5hbWVzIjpbXSwiaWdub3JlTGlzdCI6WzBdLCJzb3VyY2VSb290IjoiIn0=\n//# sourceURL=webpack-internal:///(ssr)/./node_modules/style-to-object/index.js\n");

/***/ }),

/***/ "(ssr)/./node_modules/style-to-object/index.mjs":
/*!************************************************!*\
  !*** ./node_modules/style-to-object/index.mjs ***!
  \************************************************/
/***/ ((__unused_webpack___webpack_module__, __webpack_exports__, __webpack_require__) => {

"use strict";
eval("__webpack_require__.r(__webpack_exports__);\n/* harmony export */ __webpack_require__.d(__webpack_exports__, {\n/* harmony export */   \"default\": () => (__WEBPACK_DEFAULT_EXPORT__)\n/* harmony export */ });\n/* harmony import */ var _index_js__WEBPACK_IMPORTED_MODULE_0__ = __webpack_require__(/*! ./index.js */ \"(ssr)/./node_modules/style-to-object/index.js\");\n\n\n/* harmony default export */ const __WEBPACK_DEFAULT_EXPORT__ = (_index_js__WEBPACK_IMPORTED_MODULE_0__);\n//# sourceURL=[module]\n//# sourceMappingURL=data:application/json;charset=utf-8;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiKHNzcikvLi9ub2RlX21vZHVsZXMvc3R5bGUtdG8tb2JqZWN0L2luZGV4Lm1qcyIsIm1hcHBpbmdzIjoiOzs7OztBQUF1Qzs7QUFFdkMsaUVBQWUsc0NBQWEsRUFBQyIsInNvdXJjZXMiOlsiL2hvbWUvY2hhaXpoZW5odWEvQ29kZXMvdW5jYXJ2ZS9lMmUvZXhhbXBsZXMvdHJhdmVsLXVpL25vZGVfbW9kdWxlcy9zdHlsZS10by1vYmplY3QvaW5kZXgubWpzIl0sInNvdXJjZXNDb250ZW50IjpbImltcG9ydCBTdHlsZVRvT2JqZWN0IGZyb20gJy4vaW5kZXguanMnO1xuXG5leHBvcnQgZGVmYXVsdCBTdHlsZVRvT2JqZWN0O1xuIl0sIm5hbWVzIjpbXSwiaWdub3JlTGlzdCI6WzBdLCJzb3VyY2VSb290IjoiIn0=\n//# sourceURL=webpack-internal:///(ssr)/./node_modules/style-to-object/index.mjs\n");

/***/ })

};
;