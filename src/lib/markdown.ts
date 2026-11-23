import { marked } from "marked";
import DOMPurify from "dompurify";

export const renderMarkdown = (s: string) => DOMPurify.sanitize(marked.parse(s, { async: false }) as string);
