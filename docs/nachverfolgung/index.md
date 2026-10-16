# Nachverfolgung

```{toctree}
:maxdepth: 1

graph
code
```

## Kennzahlen

| Kennzahl | Wert |
|---|---|
| Anforderungen | {need_count}`type == 'req'` |
| davon MVP | {need_count}`type == 'req' and release == 'MVP'` |
| Anforderungen mit Spezifikation | {need_count}`type == 'req' and len(satisfies_back) > 0` |
| Spezifikationen | {need_count}`type == 'spec'` |
| Spezifikationen mit Implementierung | {need_count}`type == 'spec' and len(implements_back) > 0` |
| Spezifikationen mit Test | {need_count}`type == 'spec' and len(verifies_back) > 0` |
| Architekturentscheidungen | {need_count}`type == 'adr'` |
| Code-Stellen (`@impl`) | {need_count}`type == 'impl'` |
| Tests (`@test`) | {need_count}`type == 'test'` |
| Arbeitspakete MVP | {need_count}`type == 'wp' and 'WP_MVP' in id` |
| MVP-Anforderungen mit Arbeitspaket | {need_count}`type == 'req' and release == 'MVP' and len(covers_back) > 0` |
| davon v1 | {need_count}`type == 'req' and release == 'v1'` |
| Arbeitspakete v1 | {need_count}`type == 'wp' and 'WP_V1' in id` |
| v1-Anforderungen mit Arbeitspaket | {need_count}`type == 'req' and release == 'v1' and len(covers_back) > 0` |

## Anforderung → Spezifikation

```{needtable}
:types: req
:columns: id, title, priority, release, satisfies_back as "Spezifikation", decides_back as "ADR"
:style: table
:sort: id
```

## Spezifikation → Code und Tests

```{needtable}
:types: spec
:columns: id, title, status, satisfies as "Anforderung", implements_back as "Code", verifies_back as "Tests"
:style: table
:sort: id
```

## Lücken

### MVP-Anforderungen ohne Spezifikation

```{needtable}
:filter: type == 'req' and release == 'MVP' and len(satisfies_back) == 0
:columns: id, title, priority
:style: table
```

### MVP-Anforderungen ohne Arbeitspaket

```{needtable}
:filter: type == 'req' and release == 'MVP' and len(covers_back) == 0
:columns: id, title, priority
:style: table
```

### v1-Anforderungen ohne Spezifikation

```{needtable}
:filter: type == 'req' and release == 'v1' and len(satisfies_back) == 0
:columns: id, title, priority
:style: table
```

### v1-Anforderungen ohne Arbeitspaket

```{needtable}
:filter: type == 'req' and release == 'v1' and len(covers_back) == 0
:columns: id, title, priority
:style: table
```

### Spezifikationen ohne Implementierung

```{needtable}
:filter: type == 'spec' and len(implements_back) == 0
:columns: id, title, status
:style: table
```

### Spezifikationen ohne Test

```{needtable}
:filter: type == 'spec' and len(verifies_back) == 0
:columns: id, title, status
:style: table
```

### Als `implemented` markiert, aber ohne Code

```{needtable}
:filter: type == 'spec' and status == 'implemented' and len(implements_back) == 0
:columns: id, title
:style: table
```
