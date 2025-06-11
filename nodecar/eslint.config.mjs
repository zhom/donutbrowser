import { FlatCompat } from "@eslint/eslintrc";
import eslint from "@eslint/js";
import tseslint from "typescript-eslint";

const compat = new FlatCompat({
  baseDirectory: import.meta.dirname,
});

const eslintConfig = tseslint.config(
  eslint.configs.recommended,
  ...compat.extends("next/core-web-vitals"),
  {
    // Disabled rules taken from https://biomejs.dev/linter/rules-sources for ones that
    // are already handled by Prettier and TypeScript or are not needed
    rules: {
      // eslint-plugin-jsx-a11y rules - some disabled for performance/specific project needs
      "jsx-a11y/alt-text": "off",
      "jsx-a11y/anchor-has-content": "off",
      "jsx-a11y/anchor-is-valid": "off",
      "jsx-a11y/aria-activedescendant-has-tabindex": "off",
      "jsx-a11y/aria-props": "off",
      "jsx-a11y/aria-proptypes": "off",
      "jsx-a11y/aria-role": "off",
      "jsx-a11y/aria-unsupported-elements": "off",
      "jsx-a11y/autocomplete-valid": "off",
      "jsx-a11y/click-events-have-key-events": "off",
      "jsx-a11y/heading-has-content": "off",
      "jsx-a11y/html-has-lang": "off",
      "jsx-a11y/iframe-has-title": "off",
      "jsx-a11y/img-redundant-alt": "off",
      "jsx-a11y/interactive-supports-focus": "off",
      "jsx-a11y/label-has-associated-control": "off",
      "jsx-a11y/lang": "off",
      "jsx-a11y/media-has-caption": "off",
      "jsx-a11y/mouse-events-have-key-events": "off",
      "jsx-a11y/no-access-key": "off",
      "jsx-a11y/no-aria-hidden-on-focusable": "off",
      "jsx-a11y/no-autofocus": "off",
      "jsx-a11y/no-distracting-elements": "off",
      "jsx-a11y/no-interactive-element-to-noninteractive-role": "off",
      "jsx-a11y/no-noninteractive-element-to-interactive-role": "off",
      "jsx-a11y/no-noninteractive-tabindex": "off",
      "jsx-a11y/no-redundant-roles": "off",
      "jsx-a11y/no-static-element-interactions": "off",
      "jsx-a11y/prefer-tag-over-role": "off",
      "jsx-a11y/role-has-required-aria-props": "off",
      "jsx-a11y/role-supports-aria-props": "off",
      "jsx-a11y/scope": "off",
      "jsx-a11y/tabindex-no-positive": "off",
      // eslint-plugin-react rules - some disabled for performance/specific project needs
      "react/button-has-type": "off",
      "react/jsx-boolean-value": "off",
      "react/jsx-curly-brace-presence": "off",
      "react/jsx-fragments": "off",
      "react/jsx-key": "off",
      "react/jsx-no-comment-textnodes": "off",
      "react/jsx-no-duplicate-props": "off",
      "react/jsx-no-target-blank": "off",
      "react/jsx-no-useless-fragment": "off",
      "react/no-array-index-key": "off",
      "react/no-children-prop": "off",
      "react/no-danger": "off",
      "react/no-danger-with-children": "off",
      "react/void-dom-elements-no-children": "off",
      // eslint-plugin-react-hooks rules - disabled for specific project needs
      "react-hooks/exhaustive-deps": "off",
      "react-hooks/rules-of-hooks": "off",
      // typescript-eslint rules - some handled by TypeScript compiler or disabled for project needs
      "@typescript-eslint/no-explicit-any": "off",
      "@typescript-eslint/require-await": "off",
      // Custom rules
      "@typescript-eslint/restrict-template-expressions": [
        "error",
        {
          allowNumber: true,
          allowBoolean: true,
          allowNever: true,
        },
      ],
    },
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
  }
);

export default eslintConfig;
