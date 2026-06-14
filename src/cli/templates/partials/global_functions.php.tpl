{{#each functions}}if (!function_exists('{{fqn}}')) {
    function {{name}}({{params}}): {{return_type}}
    {
{{#if is_void}}        {{../entity_fqcn}}::{{name}}(...func_get_args());
{{else}}        return {{../entity_fqcn}}::{{name}}(...func_get_args());
{{/if}}    }
}

{{/each}}