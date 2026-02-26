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

// src/components/toast/toast-provider.tsx
var toast_provider_exports = {};
__export(toast_provider_exports, {
  ToastProvider: () => ToastProvider,
  useToast: () => useToast
});
module.exports = __toCommonJS(toast_provider_exports);
var import_react = require("react");
var import_shared = require("@copilotkit/shared");
var import_jsx_runtime = require("react/jsx-runtime");
var ToastContext = (0, import_react.createContext)(void 0);
function getErrorSeverity(error) {
  if (error.severity) {
    switch (error.severity) {
      case import_shared.Severity.CRITICAL:
        return "critical";
      case import_shared.Severity.WARNING:
        return "warning";
      case import_shared.Severity.INFO:
        return "info";
      default:
        return "info";
    }
  }
  const message = error.message.toLowerCase();
  if (message.includes("api key") || message.includes("401") || message.includes("unauthorized") || message.includes("authentication") || message.includes("incorrect api key")) {
    return "critical";
  }
  return "info";
}
function getErrorColors(severity) {
  switch (severity) {
    case "critical":
      return {
        background: "#fee2e2",
        border: "#dc2626",
        text: "#7f1d1d",
        icon: "#dc2626"
      };
    case "warning":
      return {
        background: "#fef3c7",
        border: "#d97706",
        text: "#78350f",
        icon: "#d97706"
      };
    case "info":
      return {
        background: "#dbeafe",
        border: "#2563eb",
        text: "#1e3a8a",
        icon: "#2563eb"
      };
  }
}
function useToast() {
  const context = (0, import_react.useContext)(ToastContext);
  if (!context) {
    throw new Error("useToast must be used within a ToastProvider");
  }
  return context;
}
function ToastProvider({
  enabled,
  children
}) {
  const [toasts, setToasts] = (0, import_react.useState)([]);
  const [bannerError, setBannerErrorState] = (0, import_react.useState)(null);
  const removeToast = (0, import_react.useCallback)((id) => {
    setToasts((prev) => prev.filter((toast) => toast.id !== id));
  }, []);
  const addToast = (0, import_react.useCallback)(
    (toast) => {
      var _a;
      if (!enabled) {
        return;
      }
      const id = (_a = toast.id) != null ? _a : Math.random().toString(36).substring(2, 9);
      setToasts((currentToasts) => {
        if (currentToasts.find((toast2) => toast2.id === id))
          return currentToasts;
        return [...currentToasts, __spreadProps(__spreadValues({}, toast), { id })];
      });
      if (toast.duration) {
        setTimeout(() => {
          removeToast(id);
        }, toast.duration);
      }
    },
    [enabled, removeToast]
  );
  const setBannerError = (0, import_react.useCallback)(
    (error) => {
      if (!enabled && error !== null) {
        return;
      }
      setBannerErrorState(error);
    },
    [enabled]
  );
  const addGraphQLErrorsToast = (0, import_react.useCallback)((errors) => {
    console.warn("addGraphQLErrorsToast is deprecated. All errors now show as banners.");
  }, []);
  const value = {
    toasts,
    addToast,
    addGraphQLErrorsToast,
    removeToast,
    enabled,
    bannerError,
    setBannerError
  };
  return /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ToastContext.Provider, { value, children: [
    bannerError && (() => {
      const severity = getErrorSeverity(bannerError);
      const colors = getErrorColors(severity);
      return /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
        "div",
        {
          style: {
            position: "fixed",
            bottom: "20px",
            left: "50%",
            transform: "translateX(-50%)",
            zIndex: 9999,
            backgroundColor: colors.background,
            border: `1px solid ${colors.border}`,
            borderLeft: `4px solid ${colors.border}`,
            borderRadius: "8px",
            padding: "12px 16px",
            fontSize: "13px",
            boxShadow: "0 4px 12px rgba(0, 0, 0, 0.15)",
            backdropFilter: "blur(8px)",
            maxWidth: "min(90vw, 700px)",
            width: "100%",
            boxSizing: "border-box",
            overflow: "hidden"
          },
          children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(
            "div",
            {
              style: {
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
                gap: "10px"
              },
              children: [
                /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(
                  "div",
                  {
                    style: {
                      display: "flex",
                      alignItems: "center",
                      gap: "8px",
                      flex: 1,
                      minWidth: 0
                    },
                    children: [
                      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
                        "div",
                        {
                          style: {
                            width: "12px",
                            height: "12px",
                            borderRadius: "50%",
                            backgroundColor: colors.border,
                            flexShrink: 0
                          }
                        }
                      ),
                      /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(
                        "div",
                        {
                          style: {
                            display: "flex",
                            alignItems: "center",
                            gap: "10px",
                            flex: 1,
                            minWidth: 0
                          },
                          children: [
                            /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
                              "div",
                              {
                                style: {
                                  color: colors.text,
                                  lineHeight: "1.4",
                                  fontWeight: "400",
                                  fontSize: "13px",
                                  flex: 1,
                                  wordBreak: "break-all",
                                  overflowWrap: "break-word",
                                  maxWidth: "550px",
                                  overflow: "hidden",
                                  display: "-webkit-box",
                                  WebkitLineClamp: 10,
                                  WebkitBoxOrient: "vertical"
                                },
                                children: (() => {
                                  let message = bannerError.message;
                                  const jsonMatch = message.match(/'message':\s*'([^']+)'/);
                                  if (jsonMatch) {
                                    return jsonMatch[1];
                                  }
                                  message = message.split(" - ")[0];
                                  message = message.split(": Error code")[0];
                                  message = message.replace(/:\s*\d{3}$/, "");
                                  message = message.replace(/See more:.*$/g, "");
                                  message = message.trim();
                                  return message || "Configuration error occurred.";
                                })()
                              }
                            ),
                            (() => {
                              const message = bannerError.message;
                              const markdownLinkRegex = /\[([^\]]+)\]\(([^)]+)\)/g;
                              const plainUrlRegex = /(https?:\/\/[^\s)]+)/g;
                              let url = null;
                              let buttonText = "See More";
                              const markdownMatch = markdownLinkRegex.exec(message);
                              if (markdownMatch) {
                                url = markdownMatch[2];
                                buttonText = "See More";
                              } else {
                                const urlMatch = plainUrlRegex.exec(message);
                                if (urlMatch) {
                                  url = urlMatch[0].replace(/[.,;:'"]*$/, "");
                                  buttonText = "See More";
                                }
                              }
                              if (!url)
                                return null;
                              return /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
                                "button",
                                {
                                  onClick: () => window.open(url, "_blank", "noopener,noreferrer"),
                                  style: {
                                    background: colors.border,
                                    color: "white",
                                    border: "none",
                                    borderRadius: "5px",
                                    padding: "4px 10px",
                                    fontSize: "11px",
                                    fontWeight: "500",
                                    cursor: "pointer",
                                    transition: "all 0.2s ease",
                                    flexShrink: 0
                                  },
                                  onMouseEnter: (e) => {
                                    e.currentTarget.style.opacity = "0.9";
                                    e.currentTarget.style.transform = "translateY(-1px)";
                                  },
                                  onMouseLeave: (e) => {
                                    e.currentTarget.style.opacity = "1";
                                    e.currentTarget.style.transform = "translateY(0)";
                                  },
                                  children: buttonText
                                }
                              );
                            })()
                          ]
                        }
                      )
                    ]
                  }
                ),
                /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
                  "button",
                  {
                    onClick: () => setBannerError(null),
                    style: {
                      background: "transparent",
                      border: "none",
                      color: colors.text,
                      cursor: "pointer",
                      padding: "2px",
                      borderRadius: "3px",
                      fontSize: "14px",
                      lineHeight: "1",
                      opacity: 0.6,
                      transition: "all 0.2s ease",
                      flexShrink: 0
                    },
                    title: "Dismiss",
                    onMouseEnter: (e) => {
                      e.currentTarget.style.opacity = "1";
                      e.currentTarget.style.background = "rgba(0, 0, 0, 0.05)";
                    },
                    onMouseLeave: (e) => {
                      e.currentTarget.style.opacity = "0.6";
                      e.currentTarget.style.background = "transparent";
                    },
                    children: "\xD7"
                  }
                )
              ]
            }
          )
        }
      );
    })(),
    children
  ] });
}
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  ToastProvider,
  useToast
});
//# sourceMappingURL=toast-provider.js.map