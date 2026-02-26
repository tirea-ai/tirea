"use strict";
/*
 * ATTENTION: An "eval-source-map" devtool has been used.
 * This devtool is neither made for production nor for readable output files.
 * It uses "eval()" calls to create a separate source file with attached SourceMaps in the browser devtools.
 * If you are trying to read the output file, select a different devtool (https://webpack.js.org/configuration/devtool/)
 * or disable the default devtool with "devtool: false".
 * If you are looking for production-ready output files, see mode: "production" (https://webpack.js.org/configuration/mode/).
 */
exports.id = "vendor-chunks/is-alphanumerical";
exports.ids = ["vendor-chunks/is-alphanumerical"];
exports.modules = {

/***/ "(ssr)/./node_modules/is-alphanumerical/index.js":
/*!*************************************************!*\
  !*** ./node_modules/is-alphanumerical/index.js ***!
  \*************************************************/
/***/ ((module, __unused_webpack_exports, __webpack_require__) => {

eval("\n\nvar alphabetical = __webpack_require__(/*! is-alphabetical */ \"(ssr)/./node_modules/is-alphabetical/index.js\")\nvar decimal = __webpack_require__(/*! is-decimal */ \"(ssr)/./node_modules/is-decimal/index.js\")\n\nmodule.exports = alphanumerical\n\n// Check if the given character code, or the character code at the first\n// character, is alphanumerical.\nfunction alphanumerical(character) {\n  return alphabetical(character) || decimal(character)\n}\n//# sourceURL=[module]\n//# sourceMappingURL=data:application/json;charset=utf-8;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiKHNzcikvLi9ub2RlX21vZHVsZXMvaXMtYWxwaGFudW1lcmljYWwvaW5kZXguanMiLCJtYXBwaW5ncyI6IkFBQVk7O0FBRVosbUJBQW1CLG1CQUFPLENBQUMsc0VBQWlCO0FBQzVDLGNBQWMsbUJBQU8sQ0FBQyw0REFBWTs7QUFFbEM7O0FBRUE7QUFDQTtBQUNBO0FBQ0E7QUFDQSIsInNvdXJjZXMiOlsiL2hvbWUvY2hhaXpoZW5odWEvQ29kZXMvdW5jYXJ2ZS9lMmUvZXhhbXBsZXMvdHJhdmVsLXVpL25vZGVfbW9kdWxlcy9pcy1hbHBoYW51bWVyaWNhbC9pbmRleC5qcyJdLCJzb3VyY2VzQ29udGVudCI6WyIndXNlIHN0cmljdCdcblxudmFyIGFscGhhYmV0aWNhbCA9IHJlcXVpcmUoJ2lzLWFscGhhYmV0aWNhbCcpXG52YXIgZGVjaW1hbCA9IHJlcXVpcmUoJ2lzLWRlY2ltYWwnKVxuXG5tb2R1bGUuZXhwb3J0cyA9IGFscGhhbnVtZXJpY2FsXG5cbi8vIENoZWNrIGlmIHRoZSBnaXZlbiBjaGFyYWN0ZXIgY29kZSwgb3IgdGhlIGNoYXJhY3RlciBjb2RlIGF0IHRoZSBmaXJzdFxuLy8gY2hhcmFjdGVyLCBpcyBhbHBoYW51bWVyaWNhbC5cbmZ1bmN0aW9uIGFscGhhbnVtZXJpY2FsKGNoYXJhY3Rlcikge1xuICByZXR1cm4gYWxwaGFiZXRpY2FsKGNoYXJhY3RlcikgfHwgZGVjaW1hbChjaGFyYWN0ZXIpXG59XG4iXSwibmFtZXMiOltdLCJpZ25vcmVMaXN0IjpbMF0sInNvdXJjZVJvb3QiOiIifQ==\n//# sourceURL=webpack-internal:///(ssr)/./node_modules/is-alphanumerical/index.js\n");

/***/ })

};
;