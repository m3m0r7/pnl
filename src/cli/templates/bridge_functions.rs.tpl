{{#if functions}}mod native {
    use super::*;
    unsafe extern "C" {
{{#each functions}}        pub fn {{name}}({{#each params}}{{name}}: {{rust_type}}{{#unless @last}}, {{/unless}}{{/each}}){{#if has_return}} -> {{return_type}}{{/if}};
{{/each}}    }
}

{{#each functions}}#[unsafe(no_mangle)]
pub unsafe extern "C" fn {{symbol}}({{#each params}}{{name}}: {{rust_type}}{{#unless @last}}, {{/unless}}{{/each}}){{#if has_return}} -> {{return_type}}{{/if}} {
    unsafe { native::{{name}}({{#each params}}{{name}}{{#unless @last}}, {{/unless}}{{/each}}) }
}

{{/each}}{{else}}// No native functions were discovered for this bridge.
{{/if}}