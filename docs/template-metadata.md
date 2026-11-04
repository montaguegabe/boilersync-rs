# Template Metadata (`template.json`)

BoilerSync templates must include a `template.json` file at the template root.

## Overview

`template.json` currently supports:

- Inheritance: `extends` (or legacy `parent`)
- Input metadata: `variables`, `options`
- Template input defaults: `defaults`
- Runtime behavior: `children`, `hooks`, `github`, `skip_git`

Unknown keys are ignored by current CLI commands.

## Full Example

```json
{
  "extends": "acme/templates#python/base-service",
  "variables": {
    "company_name": {
      "label": "Company Name",
      "description": "Displayed in generated docs",
      "type": "string",
      "default": "Acme"
    },
    "region": ["us", "eu"],
    "service_tier": {
      "choices": ["free", "pro", "enterprise"],
      "required": true
    }
  },
  "options": {
    "with_ci": {
      "type": "boolean",
      "description": "Enable CI workflow",
      "default": true
    }
  },
  "defaults": {
    "api_package_name": "$${name_snake}_api",
    "web_package_name": "$${name_kebab}-web",
    "api_client_export_name": "$${name_camel}",
    "with_frontend": true
  },
  "children": [
    {
      "template": "acme/templates#python/worker",
      "path": "$${name_kebab}-worker",
      "condition": "with_workers",
      "variables": {
        "queue_name": "$${name_kebab}-jobs"
      },
      "name_snake": "$${name_snake}_worker",
      "name_pretty": "$${name_pretty} Worker"
    }
  ],
  "hooks": {
    "pre_init": [
      {
        "id": "deps",
        "run": "uv sync",
        "cwd": ".",
        "env": {
          "PROJECT_NAME": "$${name_kebab}"
        }
      }
    ],
    "post_init": [
      {
        "id": "format",
        "run": "uv run ruff format .",
        "allow_failure": true
      }
    ]
  },
  "github": {
    "create_repo": true,
    "private": true,
    "repo_name": "$${name_kebab}",
    "condition": "with_github"
  },
  "skip_git": false
}
```

## Key Details

### `extends` / `parent`

- Type: `string`
- Purpose: Inherit from a parent template.
- Notes: `parent` is supported for backward compatibility.

### `variables` and `options`

- Type: object keyed by field name.
- Purpose: Declare input metadata shown by `boilersync templates details`.
- Each value supports three forms:
  - Object form:
    - `label` or `prompt`: display label
    - `description`: help text
    - `type`: field type (`string`, `boolean`, etc.)
    - `required`: boolean
    - `default`: default value (implicitly makes `required: false` unless overridden)
    - `choices` (or `options` / `enum`): allowed values
  - Array form: shorthand for `choices`
  - Scalar form: shorthand for `default`

Notes:

- `variables` are also auto-discovered from `$${...}` usage in template files and `NAME_*` path placeholders.
- Built-in naming variables such as `name_snake` and `name_pretty` are exposed through the normal variable/input flow.
- If neither `--var name_snake=...` nor saved project metadata provides a name, BoilerSync infers `name_snake` from the target directory name.
- A trailing `-workspace` / `_workspace` suffix is stripped during default project-name inference. For example, `woo-score-workspace` defaults to `name_snake=woo_score`.

Relative `extends` values are resolved within the same template source repository. Example:

```json
{
  "extends": "pip-package"
}
```

When used from `acme/templates#cli`, this resolves to `acme/templates#pip-package`.

### `defaults`

- Type: object keyed by interpolation variable name.
- Purpose: Provide template-owned defaults before missing-variable collection.
- Values: Scalars, arrays, and objects. String values support `$${...}` interpolation.
- Precedence: Existing values win. BoilerSync does not overwrite values from explicit `--var` flags, saved `.boilersync` project metadata, or defaults already applied by an earlier template in the inheritance chain.
- Persistence: Applied defaults are saved in generated project `.boilersync` metadata under `variables`.

Example:

```json
{
  "defaults": {
    "api_package_name": "$${name_snake}_api",
    "django_app_name": "$${name_snake}",
    "web_package_name": "$${name_kebab}-web",
    "api_client_package_name": "$${name_kebab}-api-client",
    "api_client_export_name": "$${name_camel}",
    "cdn_base_url": "https://cdn.openbase.app/$${name_kebab}/",
    "with_frontend": true
  }
}
```

Defaults are the main mechanism for making a template usable with one non-interactive command:

```bash
mkdir my-app-workspace
cd my-app-workspace
boilersync init acme/templates#django-react-workspace --non-interactive
```

### Automatic Environment Defaults

BoilerSync can populate a small set of variables from the local environment before prompting:

- `github_user`: when a template references this variable and no value is already set, BoilerSync runs `gh api user --jq .login`.

If automatic lookup fails, the variable remains missing. Interactive init prompts for it, while `--non-interactive` fails and asks for an explicit `--var github_user=...`.

### `children`

- Type: list of objects.
- Purpose: Initialize child projects after parent project init.
- Child object keys:
  - `template` (required): child template ref
  - `path` (required): target child directory (supports interpolation)
  - `condition` (optional): boolean expression
  - `variables` (optional): object of variables passed to child init
  - `name_snake` (optional): child project name override
  - `name_pretty` (optional): child pretty name override

### `hooks`

- Type: object with optional `pre_init` and `post_init` lists.
- Purpose: Run shell commands before/after init.
- Hook step keys:
  - `id` (optional): identifier for logs
  - `run` (required): shell command
  - `condition` (optional): boolean expression
  - `cwd` (optional): run directory relative to target dir
  - `env` (optional): key/value env map (values support interpolation)
  - `allow_failure` (optional, default `false`)

### `github`

- Type: object.
- Purpose: Optional `gh`-based repository creation during init.
- Keys:
  - `create_repo` (boolean)
  - `condition` (optional)
  - `repo_name` (default: `$${name_kebab}`)
  - `private` (default: `true`)

### `skip_git`

- Type: boolean.
- Purpose: If `true` on any template in the inheritance chain, BoilerSync skips git initialization in generated projects.
