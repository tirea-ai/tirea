//! OpenUI Lang format preset.

/// Activity type for OpenUI Lang streaming events.
pub const ACTIVITY_TYPE: &str = "generative-ui.openui-lang";

/// Returns a system prompt instructing the sub-agent to output OpenUI Lang.
pub fn system_prompt(catalog_prompt: &str) -> String {
    format!(
        "You are a UI generation agent. Output ONLY OpenUI Lang — a compact, \
         line-oriented DSL for declarative UI.\n\n\
         ## Syntax\n\
         Every line: `identifier = ComponentType(arg1, arg2)`\n\
         First line must assign to `root`.\n\
         Use `[child1, child2]` for children references.\n\
         Forward references are allowed.\n\n\
         ## Component Library\n\
         {catalog_prompt}\n\n\
         Output ONLY the OpenUI Lang code. No markdown, no explanation."
    )
}
