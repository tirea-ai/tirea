"use strict";
var __defProp = Object.defineProperty;
var __defProps = Object.defineProperties;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropDescs = Object.getOwnPropertyDescriptors;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __getOwnPropSymbols = Object.getOwnPropertySymbols;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __propIsEnum = Object.prototype.propertyIsEnumerable;
var __defNormalProp = (obj, key, value) => key in obj ? __defProp(obj, key, { enumerable: true, configurable: true, writable: true, value }) : obj[key] = value;
var __spreadValues = (a, b) => {
  for (var prop in b || (b = {}))
    if (__hasOwnProp.call(b, prop))
      __defNormalProp(a, prop, b[prop]);
  if (__getOwnPropSymbols)
    for (var prop of __getOwnPropSymbols(b)) {
      if (__propIsEnum.call(b, prop))
        __defNormalProp(a, prop, b[prop]);
    }
  return a;
};
var __spreadProps = (a, b) => __defProps(a, __getOwnPropDescs(b));
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

// src/hooks/use-tree.ts
var use_tree_exports = {};
__export(use_tree_exports, {
  default: () => use_tree_default
});
module.exports = __toCommonJS(use_tree_exports);
var import_shared = require("@copilotkit/shared");
var import_react = require("react");
var removeNode = (nodes, id) => {
  return nodes.reduce((result, node) => {
    if (node.id !== id) {
      const newNode = __spreadProps(__spreadValues({}, node), { children: removeNode(node.children, id) });
      result.push(newNode);
    }
    return result;
  }, []);
};
var addNode = (nodes, newNode, parentId) => {
  if (!parentId) {
    return [...nodes, newNode];
  }
  return nodes.map((node) => {
    if (node.id === parentId) {
      return __spreadProps(__spreadValues({}, node), { children: [...node.children, newNode] });
    } else if (node.children.length) {
      return __spreadProps(__spreadValues({}, node), { children: addNode(node.children, newNode, parentId) });
    }
    return node;
  });
};
var treeIndentationRepresentation = (index, indentLevel) => {
  if (indentLevel === 0) {
    return (index + 1).toString();
  } else if (indentLevel === 1) {
    return String.fromCharCode(65 + index);
  } else if (indentLevel === 2) {
    return String.fromCharCode(97 + index);
  } else {
    return "-";
  }
};
var printNode = (node, prefix = "", indentLevel = 0) => {
  const indent = " ".repeat(3).repeat(indentLevel);
  const prefixPlusIndentLength = prefix.length + indent.length;
  const subsequentLinesPrefix = " ".repeat(prefixPlusIndentLength);
  const valueLines = node.value.split("\n");
  const outputFirstLine = `${indent}${prefix}${valueLines[0]}`;
  const outputSubsequentLines = valueLines.slice(1).map((line) => `${subsequentLinesPrefix}${line}`).join("\n");
  let output = `${outputFirstLine}
`;
  if (outputSubsequentLines) {
    output += `${outputSubsequentLines}
`;
  }
  const childPrePrefix = " ".repeat(prefix.length);
  node.children.forEach(
    (child, index) => output += printNode(
      child,
      `${childPrePrefix}${treeIndentationRepresentation(index, indentLevel + 1)}. `,
      indentLevel + 1
    )
  );
  return output;
};
function treeReducer(state, action) {
  switch (action.type) {
    case "ADD_NODE": {
      const { value, parentId, id: newNodeId } = action;
      const newNode = {
        id: newNodeId,
        value,
        children: [],
        categories: new Set(action.categories)
      };
      try {
        return addNode(state, newNode, parentId);
      } catch (error) {
        console.error(`Error while adding node with id ${newNodeId}: ${error}`);
        return state;
      }
    }
    case "REMOVE_NODE":
      return removeNode(state, action.id);
    default:
      return state;
  }
}
var useTree = () => {
  const [tree, dispatch] = (0, import_react.useReducer)(treeReducer, []);
  const addElement = (0, import_react.useCallback)(
    (value, categories, parentId) => {
      const newNodeId = (0, import_shared.randomId)();
      dispatch({
        type: "ADD_NODE",
        value,
        parentId,
        id: newNodeId,
        categories
      });
      return newNodeId;
    },
    []
  );
  const removeElement = (0, import_react.useCallback)((id) => {
    dispatch({ type: "REMOVE_NODE", id });
  }, []);
  const getAllElements = (0, import_react.useCallback)(() => {
    return tree;
  }, [tree]);
  const printTree = (0, import_react.useCallback)(
    (categories) => {
      const categoriesSet = new Set(categories);
      let output = "";
      tree.forEach((node, index) => {
        if (!setsHaveIntersection(categoriesSet, node.categories)) {
          return;
        }
        if (index !== 0) {
          output += "\n";
        }
        output += printNode(node, `${treeIndentationRepresentation(index, 0)}. `);
      });
      return output;
    },
    [tree]
  );
  return { tree, addElement, printTree, removeElement, getAllElements };
};
var use_tree_default = useTree;
function setsHaveIntersection(setA, setB) {
  const [smallerSet, largerSet] = setA.size <= setB.size ? [setA, setB] : [setB, setA];
  for (let item of smallerSet) {
    if (largerSet.has(item)) {
      return true;
    }
  }
  return false;
}
//# sourceMappingURL=use-tree.js.map