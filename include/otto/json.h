#ifndef OTTO_JSON_H
#define OTTO_JSON_H

#include <stddef.h>

#include "otto/common.h"

typedef struct {
    char *content;
    char *error_message;
    char *error_type;
} OttoJsonResult;

OttoExitCode otto_json_build_request(
    const char *model,
    const char *system_prompt,
    const char *prompt,
    int stream,
    char **json,
    size_t *json_length
);

OttoExitCode otto_json_parse_stream_event(
    const char *json,
    size_t length,
    char **content
);

OttoExitCode otto_json_parse_response(
    const char *json,
    size_t length,
    OttoJsonResult *result
);

void otto_json_result_free(OttoJsonResult *result);

#endif
