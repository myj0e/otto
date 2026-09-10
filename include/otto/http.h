#ifndef OTTO_HTTP_H
#define OTTO_HTTP_H

#include <stddef.h>

#include "otto/common.h"

typedef struct {
    const char *endpoint;
    const char *apikey;
    const char *body;
    size_t body_length;
    long connect_timeout_ms;
    long timeout_ms;
    size_t max_response_size;
} OttoHttpRequest;

typedef struct {
    long http_status;
    char *body;
    size_t body_length;
    char error_message[OTTO_CURL_ERROR_SIZE];
} OttoHttpResponse;

OttoExitCode otto_http_global_init(void);
void otto_http_global_cleanup(void);
void otto_http_response_init(OttoHttpResponse *response);
void otto_http_response_free(OttoHttpResponse *response);
OttoExitCode otto_http_chat(
    const OttoHttpRequest *request,
    OttoHttpResponse *response
);

#endif
