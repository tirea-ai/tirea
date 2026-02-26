// src/components/toast/exclamation-mark-icon.tsx
import { jsx, jsxs } from "react/jsx-runtime";
var ExclamationMarkIcon = ({
  className,
  style
}) => /* @__PURE__ */ jsxs(
  "svg",
  {
    xmlns: "http://www.w3.org/2000/svg",
    width: "24",
    height: "24",
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: "2",
    strokeLinecap: "round",
    strokeLinejoin: "round",
    className: `lucide lucide-circle-alert ${className ? className : ""}`,
    style,
    children: [
      /* @__PURE__ */ jsx("circle", { cx: "12", cy: "12", r: "10" }),
      /* @__PURE__ */ jsx("line", { x1: "12", x2: "12", y1: "8", y2: "12" }),
      /* @__PURE__ */ jsx("line", { x1: "12", x2: "12.01", y1: "16", y2: "16" })
    ]
  }
);

export {
  ExclamationMarkIcon
};
//# sourceMappingURL=chunk-O7ARI5CV.mjs.map