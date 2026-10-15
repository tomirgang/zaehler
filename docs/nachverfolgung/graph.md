# Trace-Graph

Benötigt Graphviz (`dot`); ohne Graphviz wird diese Seite beim Build ausgelassen.

## Verbrauch und Prognose

```{needflow}
:filter: id in ['REQ_ANA_001', 'REQ_MTR_005', 'REQ_MTR_006', 'REQ_FC_001', 'REQ_ANA_002'] or (type == 'spec' and ('SPEC_CONS' in id or 'SPEC_FC' in id)) or (type in ['impl', 'test'] and ('consumption' in title or 'forecast' in title))
:show_link_names:
```

## Betrieb

```{needflow}
:filter: (type == 'req' and 'REQ_OPS' in id) or (type == 'spec' and 'SPEC_OPS' in id)
:show_link_names:
```
