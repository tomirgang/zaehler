{{- define "zaehler.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "zaehler.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{- define "zaehler.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" }}
{{ include "zaehler.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "zaehler.selectorLabels" -}}
app.kubernetes.io/name: {{ include "zaehler.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "zaehler.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "zaehler.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "zaehler.cnpgCluster" -}}
{{- printf "%s-db" (include "zaehler.fullname" .) }}
{{- end }}

{{/* Benötigt ein PVC für /var/lib/zaehler/data? */}}
{{- define "zaehler.needsDataVolume" -}}
{{- if or (eq .Values.database.type "sqlite") (eq .Values.storage.kind "fs") }}true{{- end }}
{{- end }}

{{- define "zaehler.validate" -}}
{{- if not (has .Values.database.type (list "cnpg" "existing" "sqlite")) }}
{{- fail "database.type muss cnpg, existing oder sqlite sein" }}
{{- end }}
{{- if not .Values.oidc.enabled }}
{{- fail "oidc.enabled=true mit oidc.issuerUrl und oidc.existingSecret erforderlich" }}
{{- end }}
{{- if and .Values.oidc.enabled (or (not .Values.oidc.issuerUrl) (not .Values.oidc.existingSecret)) }}
{{- fail "oidc.issuerUrl und oidc.existingSecret sind erforderlich" }}
{{- end }}
{{- if and (eq .Values.database.type "existing") (not .Values.database.existing.secretName) }}
{{- fail "database.existing.secretName ist erforderlich" }}
{{- end }}
{{- if and (eq .Values.storage.kind "fs") (ne .Values.database.type "sqlite") (gt (int .Values.replicaCount) 1) }}
{{- fail "storage.kind=fs mit mehreren Replicas benötigt ReadWriteMany; storage.kind=s3 oder replicaCount=1 setzen" }}
{{- end }}
{{- if and .Values.database.cnpg.scheduledBackup.enabled (not .Values.database.cnpg.backup) }}
{{- fail "database.cnpg.scheduledBackup.enabled setzt database.cnpg.backup voraus" }}
{{- end }}
{{- end }}
