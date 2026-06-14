{{#each functions}}if (!function_exists('{{fqn}}')) {
    function {{name}}({{params}}): {{return_type}}
    {
{{#if is_void}}        $GLOBALS['{{../runtime_var}}']->{'{{name}}'}(...func_get_args());
{{else}}        return $GLOBALS['{{../runtime_var}}']->{'{{name}}'}(...func_get_args());
{{/if}}    }
}

{{/each}}