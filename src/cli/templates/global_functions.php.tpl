{{#each functions}}if (!function_exists('{{fqn}}')) {
    function {{name}}({{#each params}}{{php_type}} ${{name}}{{#unless @last}}, {{/unless}}{{/each}}): {{return_type}}
    {
{{#if is_void}}        $GLOBALS['{{../runtime_var}}']->{'{{name}}'}(...func_get_args());
{{else}}        return $GLOBALS['{{../runtime_var}}']->{'{{name}}'}(...func_get_args());
{{/if}}    }
}

{{/each}}