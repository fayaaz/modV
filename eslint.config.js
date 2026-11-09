const globals = require("globals");
const electronToolkit = require("@electron-toolkit/eslint-config");
const pluginVue = require("eslint-plugin-vue");
const prettier = require("@vue/eslint-config-prettier");

module.exports = [
  {
    ignores: [
      "node_modules/**",
      "dist/**",
      "dist_electron/**",
      "out/**",
      "test-results/**",
      "tmp/**",
      ".opencode/**",
      "src/renderer/src/v3-gl-ext.es.js",
    ],
  },
  electronToolkit,
  ...pluginVue.configs["flat/recommended"],
  prettier,
  {
    languageOptions: {
      ecmaVersion: 2023,
      sourceType: "module",
    },
    rules: {
      "no-unused-vars": [
        "error",
        {
          args: "after-used",
          argsIgnorePattern: "^_",
          caughtErrors: "none",
        },
      ],
      // New in ESLint 9/10: noisy on the vendored/minified bundles and on the
      // existing render loops. Keep the pre-upgrade behaviour.
      "no-useless-assignment": "off",
      "no-constant-binary-expression": "off",
      "preserve-caught-error": "off",
      "vue/require-default-prop": "off",
      "vue/multi-word-component-names": "off",
    },
  },
  {
    files: ["src/renderer/src/application/worker/**/*.js"],
    languageOptions: {
      globals: {
        ...globals.worker,
      },
    },
  },
];
