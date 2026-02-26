// src/hooks/use-dark-mode.ts
var useDarkMode = () => {
  if (typeof window === "undefined")
    return false;
  return document.documentElement.classList.contains("dark") || document.body.classList.contains("dark") || document.documentElement.getAttribute("data-theme") === "dark" || document.body.getAttribute("data-theme") === "dark" || window.matchMedia("(prefers-color-scheme: dark)").matches;
};

export {
  useDarkMode
};
//# sourceMappingURL=chunk-JGMFJZMG.mjs.map