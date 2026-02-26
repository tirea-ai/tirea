type FlatCategoryStoreId = string;
interface UseFlatCategoryStoreReturn<T> {
    addElement: (value: T, categories: string[]) => FlatCategoryStoreId;
    removeElement: (id: FlatCategoryStoreId) => void;
    allElements: (categories: string[]) => T[];
}
declare const useFlatCategoryStore: <T>() => UseFlatCategoryStoreReturn<T>;

export { FlatCategoryStoreId, UseFlatCategoryStoreReturn, useFlatCategoryStore as default };
