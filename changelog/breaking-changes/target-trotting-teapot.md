Select generated APIs with `--targets workflow`, `--targets client`, or both; `--native-api` is removed and the default emits only models and definitions. Python client generation no longer requires a workflow API.
Sources apply to all API targets unless restricted with `targets="..."`. Shared models no longer evaluate source defaults; resources and sourced fields require at least one API target.
