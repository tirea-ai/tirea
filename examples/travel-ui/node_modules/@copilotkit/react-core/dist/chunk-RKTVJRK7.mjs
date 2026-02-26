import {
  __spreadProps,
  __spreadValues
} from "./chunk-SKC7AJIV.mjs";

// src/hooks/use-tree.ts
import { randomId } from "@copilotkit/shared";
import { useCallback, useReducer } from "react";
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
  const [tree, dispatch] = useReducer(treeReducer, []);
  const addElement = useCallback(
    (value, categories, parentId) => {
      const newNodeId = randomId();
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
  const removeElement = useCallback((id) => {
    dispatch({ type: "REMOVE_NODE", id });
  }, []);
  const getAllElements = useCallback(() => {
    return tree;
  }, [tree]);
  const printTree = useCallback(
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

export {
  use_tree_default
};
//# sourceMappingURL=chunk-RKTVJRK7.mjs.map