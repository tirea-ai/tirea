import {
  __async
} from "../chunk-MRXNTQOX.mjs";

// src/lib/utils.ts
function fetcher(input, init) {
  return __async(this, null, function* () {
    const res = yield fetch(input, init);
    if (!res.ok) {
      const json = yield res.json();
      if (json.error) {
        const error = new Error(json.error);
        error.status = res.status;
        throw error;
      } else {
        throw new Error("An unexpected error occurred");
      }
    }
    return res.json();
  });
}
function formatDate(input) {
  const date = new Date(input);
  return date.toLocaleDateString("en-US", {
    month: "long",
    day: "numeric",
    year: "numeric"
  });
}
export {
  fetcher,
  formatDate
};
//# sourceMappingURL=utils.mjs.map