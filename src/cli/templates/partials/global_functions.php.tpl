{{~#*inline "param_type"~}}
{{#if is_string}}string|\Pnlx\Helpers\String_|\Stringable|null{{/if}}{{#if is_int}}int|\Pnlx\Helpers\AnySizeInteger{{/if}}{{#if is_float}}float|int|\Pnlx\Helpers\AnyFloat{{/if}}{{#if is_pointer}}{{#if pointer_class}}{{pointer_class}}|{{/if}}\Pnlx\Helpers\ContextInterface|null{{/if}}{{#if cdata}}|\FFI\CData{{/if}}
{{~/inline~}}
{{#each functions}}if (!function_exists('{{fqn}}')) {
    function {{name}}({{#each params}}{{> param_type}} ${{name}}{{#unless @last}}, {{/unless}}{{/each}}): {{#if is_void}}void{{else}}{{#if return_native}}{{return_native}}|{{/if}}{{return_class}}{{/if}}
    {
{{#if is_void}}        {{../entity_fqcn}}::{{name}}(...func_get_args());
{{else}}        return {{../entity_fqcn}}::{{name}}(...func_get_args());
{{/if}}    }
}

{{/each}}
