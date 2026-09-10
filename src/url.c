#include "otto/url.h"

#include <ctype.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>

#include "otto/util.h"

static int has_suffix(const char *value, const char *suffix)
{
    size_t value_length;
    size_t suffix_length;

    value_length = strlen(value);
    suffix_length = strlen(suffix);
    return value_length >= suffix_length &&
           strcmp(value + value_length - suffix_length, suffix) == 0;
}

static char *append_path(const char *base, const char *path)
{
    size_t base_length = strlen(base);
    size_t path_length = strlen(path);
    size_t total_length;
    char *result;

    if (base_length > SIZE_MAX - path_length) {
        return NULL;
    }
    total_length = base_length + path_length;
    if (total_length == SIZE_MAX) {
        return NULL;
    }

    result = malloc(total_length + 1U);
    if (result == NULL) {
        return NULL;
    }

    memcpy(result, base, base_length);
    memcpy(result + base_length, path, path_length + 1U);
    return result;
}

OttoExitCode otto_build_endpoint(const char *baseurl, char **endpoint)
{
    char *trimmed;
    size_t length;
    const char *host;
    size_t scheme_length;
    int is_http;
    int is_https;

    if (baseurl == NULL || endpoint == NULL) {
        return OTTO_ERR_CONFIG;
    }

    *endpoint = NULL;
    trimmed = otto_trim_copy(baseurl);
    if (trimmed == NULL) {
        return OTTO_ERR_MEMORY;
    }

    length = strlen(trimmed);
    while (length > 0U && trimmed[length - 1U] == '/') {
        trimmed[--length] = '\0';
    }

    is_http = strncasecmp(trimmed, "http://", 7U) == 0;
    is_https = strncasecmp(trimmed, "https://", 8U) == 0;
    if (!is_http && !is_https) {
        fprintf(stderr, "otto: baseurl 必须是有效的 HTTP/HTTPS 地址\n");
        free(trimmed);
        return OTTO_ERR_CONFIG;
    }

    scheme_length = is_https ? 8U : 7U;
    host = trimmed + scheme_length;
    if (host[0] == '\0' || host[0] == '/' ||
        strchr(trimmed, '?') != NULL || strchr(trimmed, '#') != NULL) {
        fprintf(stderr, "otto: baseurl 必须是有效的 HTTP/HTTPS 地址\n");
        free(trimmed);
        return OTTO_ERR_CONFIG;
    }

    for (length = 0U; trimmed[length] != '\0'; length++) {
        if (isspace((unsigned char)trimmed[length])) {
            fprintf(stderr, "otto: baseurl 不能包含空白字符\n");
            free(trimmed);
            return OTTO_ERR_CONFIG;
        }
    }

    if (has_suffix(trimmed, "/chat/completions")) {
        *endpoint = trimmed;
        return OTTO_OK;
    }

    if (has_suffix(trimmed, "/v1")) {
        *endpoint = append_path(trimmed, "/chat/completions");
    } else {
        *endpoint = append_path(trimmed, "/v1/chat/completions");
    }

    free(trimmed);
    if (*endpoint == NULL) {
        return OTTO_ERR_MEMORY;
    }

    return OTTO_OK;
}
