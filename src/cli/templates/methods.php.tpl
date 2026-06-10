{{#each methods}}    public function {{name}}({{#each params}}{{php_type}} ${{name}}{{#unless @last}}, {{/unless}}{{/each}}): {{return_type}}
    {
{{#if is_void}}        $this->__call('{{dispatch}}', func_get_args());
{{else}}        return {{#if cstring}}\Pnlx\Util::cString($this->__call('{{dispatch}}', func_get_args())){{else}}{{cast}}$this->__call('{{dispatch}}', func_get_args()){{/if}};
{{/if}}    }

{{/each}}