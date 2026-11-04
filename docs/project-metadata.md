# Project Metadata (`.boilersync`)

BoilerSync writes a `.boilersync` file in the project root after `boilersync init` and `boilersync pull`.

This file is the source of truth for template provenance and saved interpolation inputs.

BoilerSync writes this file as indented JSON with a trailing newline.

## Schema

```json
{
  "template": "https://github.com/acme/templates.git#python/service-template",
  "template_commit": "abc123def4567890abc123def4567890abc123de",
  "name_snake": "my_service",
  "name_pretty": "My Service",
  "variables": {
    "description": "Example service"
  },
  "children": ["my-service-worker"]
}
```

## Field-By-Field Reference

### `template` (required)

- Type: `string`
- Format: `https://github.com/<org>/<repo>(.git optional)#<template_subdir>`
- Notes:
  - GitHub host is required.
  - `<template_subdir>` is required and cannot be empty.
  - BoilerSync canonicalizes this value to include `.git` when writing the file.
  - The source repository is resolved/cloned at:
    - `${BOILERSYNC_TEMPLATE_DIR:-~/.boilersync/templates}/<org>/<repo>`

### `name_snake` (required)

- Type: `string`
- Purpose: project identifier used for interpolation (`$${name_snake}`).
- Notes: callers now pass this through the normal template variable interface (for example `--var name_snake=my_service`).

### `name_pretty` (required)

- Type: `string`
- Purpose: display name used for interpolation (`$${name_pretty}`).
- Notes: callers now pass this through the normal template variable interface (for example `--var name_pretty="My Service"`).

### `template_commit` (required for new projects)

- Type: `string`
- Purpose: the source template repository commit that the project last pulled from.
- Notes:
  - BoilerSync records the current HEAD commit of the cached template repository.
  - `boilersync check-pull` compares this stored commit against the current cached template repo HEAD.
  - Older projects may not have this field until they run `boilersync pull` again.

### `variables` (required)

- Type: `object`
- Purpose: saved interpolation variables for repeatable `pull` and `push` flows.
- Notes:
  - Keys/values are template-specific.
  - Values populated from `template.json` `defaults` are stored here after init or pull.
  - Explicit values such as `--var KEY=VALUE` take precedence over template defaults.

### `children` (optional)

- Type: `string[]`
- Purpose: relative paths to explicit child BoilerSync projects.
- Notes:
  - Paths are relative to the directory containing `.boilersync`.
  - Field is present when child projects are registered.
  - Parent diffs exclude registered child paths so each template instantiation is compared against its own upstream template.
  - BoilerSync does not infer children from framework conventions, package names, or folder names. Unregistered extra folders remain part of the parent project diff.

## Validation Rules

- Missing or invalid `template` fails `.boilersync` resolution.
- Legacy metadata shapes are not supported.
- Unknown extra keys are ignored by current commands.
