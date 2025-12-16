# fask

todo finder for your codebase

## commands

### fask current

search todos in current files

```
--pattern <PATTERN>     pattern to search [default: TODO]
-C, --context <N>       context lines [default: 2]
-t, --file-type <TYPE>  file pattern (e.g., *.rs)
-d, --directory <DIR>   file directory [default: .]
```

### fask since

search todos added after a date (git history)

```
--date <DATE>           date in yyyy-mm-dd format [required]
--pattern <PATTERN>     pattern to search [default: TODO]
-C, --context <N>       context lines [default: 2]
-D, --directory <DIR>   directory [default: .]
```

## examples

```bash
fask current
fask current --pattern FIXME --context 5
fask since --date "2025-12-01"
```
