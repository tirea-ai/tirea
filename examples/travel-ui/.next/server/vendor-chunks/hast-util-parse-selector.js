"use strict";
/*
 * ATTENTION: An "eval-source-map" devtool has been used.
 * This devtool is neither made for production nor for readable output files.
 * It uses "eval()" calls to create a separate source file with attached SourceMaps in the browser devtools.
 * If you are trying to read the output file, select a different devtool (https://webpack.js.org/configuration/devtool/)
 * or disable the default devtool with "devtool: false".
 * If you are looking for production-ready output files, see mode: "production" (https://webpack.js.org/configuration/mode/).
 */
exports.id = "vendor-chunks/hast-util-parse-selector";
exports.ids = ["vendor-chunks/hast-util-parse-selector"];
exports.modules = {

/***/ "(ssr)/./node_modules/hast-util-parse-selector/index.js":
/*!********************************************************!*\
  !*** ./node_modules/hast-util-parse-selector/index.js ***!
  \********************************************************/
/***/ ((module) => {

eval("\n\nmodule.exports = parse\n\nvar search = /[#.]/g\n\n// Create a hast element from a simple CSS selector.\nfunction parse(selector, defaultTagName) {\n  var value = selector || ''\n  var name = defaultTagName || 'div'\n  var props = {}\n  var start = 0\n  var subvalue\n  var previous\n  var match\n\n  while (start < value.length) {\n    search.lastIndex = start\n    match = search.exec(value)\n    subvalue = value.slice(start, match ? match.index : value.length)\n\n    if (subvalue) {\n      if (!previous) {\n        name = subvalue\n      } else if (previous === '#') {\n        props.id = subvalue\n      } else if (props.className) {\n        props.className.push(subvalue)\n      } else {\n        props.className = [subvalue]\n      }\n\n      start += subvalue.length\n    }\n\n    if (match) {\n      previous = match[0]\n      start++\n    }\n  }\n\n  return {type: 'element', tagName: name, properties: props, children: []}\n}\n//# sourceURL=[module]\n//# sourceMappingURL=data:application/json;charset=utf-8;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoiKHNzcikvLi9ub2RlX21vZHVsZXMvaGFzdC11dGlsLXBhcnNlLXNlbGVjdG9yL2luZGV4LmpzIiwibWFwcGluZ3MiOiJBQUFZOztBQUVaOztBQUVBOztBQUVBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTs7QUFFQTtBQUNBO0FBQ0E7QUFDQTs7QUFFQTtBQUNBO0FBQ0E7QUFDQSxRQUFRO0FBQ1I7QUFDQSxRQUFRO0FBQ1I7QUFDQSxRQUFRO0FBQ1I7QUFDQTs7QUFFQTtBQUNBOztBQUVBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7O0FBRUEsVUFBVTtBQUNWIiwic291cmNlcyI6WyIvaG9tZS9jaGFpemhlbmh1YS9Db2Rlcy91bmNhcnZlL2UyZS9leGFtcGxlcy90cmF2ZWwtdWkvbm9kZV9tb2R1bGVzL2hhc3QtdXRpbC1wYXJzZS1zZWxlY3Rvci9pbmRleC5qcyJdLCJzb3VyY2VzQ29udGVudCI6WyIndXNlIHN0cmljdCdcblxubW9kdWxlLmV4cG9ydHMgPSBwYXJzZVxuXG52YXIgc2VhcmNoID0gL1sjLl0vZ1xuXG4vLyBDcmVhdGUgYSBoYXN0IGVsZW1lbnQgZnJvbSBhIHNpbXBsZSBDU1Mgc2VsZWN0b3IuXG5mdW5jdGlvbiBwYXJzZShzZWxlY3RvciwgZGVmYXVsdFRhZ05hbWUpIHtcbiAgdmFyIHZhbHVlID0gc2VsZWN0b3IgfHwgJydcbiAgdmFyIG5hbWUgPSBkZWZhdWx0VGFnTmFtZSB8fCAnZGl2J1xuICB2YXIgcHJvcHMgPSB7fVxuICB2YXIgc3RhcnQgPSAwXG4gIHZhciBzdWJ2YWx1ZVxuICB2YXIgcHJldmlvdXNcbiAgdmFyIG1hdGNoXG5cbiAgd2hpbGUgKHN0YXJ0IDwgdmFsdWUubGVuZ3RoKSB7XG4gICAgc2VhcmNoLmxhc3RJbmRleCA9IHN0YXJ0XG4gICAgbWF0Y2ggPSBzZWFyY2guZXhlYyh2YWx1ZSlcbiAgICBzdWJ2YWx1ZSA9IHZhbHVlLnNsaWNlKHN0YXJ0LCBtYXRjaCA/IG1hdGNoLmluZGV4IDogdmFsdWUubGVuZ3RoKVxuXG4gICAgaWYgKHN1YnZhbHVlKSB7XG4gICAgICBpZiAoIXByZXZpb3VzKSB7XG4gICAgICAgIG5hbWUgPSBzdWJ2YWx1ZVxuICAgICAgfSBlbHNlIGlmIChwcmV2aW91cyA9PT0gJyMnKSB7XG4gICAgICAgIHByb3BzLmlkID0gc3VidmFsdWVcbiAgICAgIH0gZWxzZSBpZiAocHJvcHMuY2xhc3NOYW1lKSB7XG4gICAgICAgIHByb3BzLmNsYXNzTmFtZS5wdXNoKHN1YnZhbHVlKVxuICAgICAgfSBlbHNlIHtcbiAgICAgICAgcHJvcHMuY2xhc3NOYW1lID0gW3N1YnZhbHVlXVxuICAgICAgfVxuXG4gICAgICBzdGFydCArPSBzdWJ2YWx1ZS5sZW5ndGhcbiAgICB9XG5cbiAgICBpZiAobWF0Y2gpIHtcbiAgICAgIHByZXZpb3VzID0gbWF0Y2hbMF1cbiAgICAgIHN0YXJ0KytcbiAgICB9XG4gIH1cblxuICByZXR1cm4ge3R5cGU6ICdlbGVtZW50JywgdGFnTmFtZTogbmFtZSwgcHJvcGVydGllczogcHJvcHMsIGNoaWxkcmVuOiBbXX1cbn1cbiJdLCJuYW1lcyI6W10sImlnbm9yZUxpc3QiOlswXSwic291cmNlUm9vdCI6IiJ9\n//# sourceURL=webpack-internal:///(ssr)/./node_modules/hast-util-parse-selector/index.js\n");

/***/ })

};
;