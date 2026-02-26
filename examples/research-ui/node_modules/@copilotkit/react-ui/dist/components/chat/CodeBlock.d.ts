import { FC } from 'react';

interface Props {
    language: string;
    value: string;
}
interface languageMap {
    [key: string]: string | undefined;
}
declare const programmingLanguages: languageMap;
declare const generateRandomString: (length: number, lowercase?: boolean) => string;
declare const CodeBlock: FC<Props>;

export { CodeBlock, generateRandomString, programmingLanguages };
