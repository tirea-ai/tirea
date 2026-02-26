interface DocumentPointer {
    id: string;
    name: string;
    sourceApplication: string;
    iconImageUri: string;
    getContents: () => string;
}

export { DocumentPointer };
