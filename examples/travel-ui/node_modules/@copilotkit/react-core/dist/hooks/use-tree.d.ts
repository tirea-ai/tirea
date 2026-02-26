type TreeNodeId = string;
interface TreeNode {
    id: TreeNodeId;
    value: string;
    children: TreeNode[];
    parentId?: TreeNodeId;
    categories: Set<string>;
}
type Tree = TreeNode[];
interface UseTreeReturn {
    tree: Tree;
    addElement: (value: string, categories: string[], parentId?: TreeNodeId) => TreeNodeId;
    printTree: (categories: string[]) => string;
    removeElement: (id: TreeNodeId) => void;
    getAllElements: () => Tree;
}
declare const useTree: () => UseTreeReturn;

export { Tree, TreeNode, TreeNodeId, UseTreeReturn, useTree as default };
