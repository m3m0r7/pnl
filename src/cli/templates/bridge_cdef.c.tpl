typedef unsigned long size_t;
typedef signed long ssize_t;

{{#each functions}}{{return_type}} {{symbol}}({{#if params}}{{#each params}}{{c_type}} {{name}}{{#unless @last}}, {{/unless}}{{/each}}{{else}}void{{/if}});
{{/each}}