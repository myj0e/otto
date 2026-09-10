#ifndef OTTO_COMMON_H
#define OTTO_COMMON_H

#include <stddef.h>

#define OTTO_VERSION "0.1.0"
#define OTTO_DEFAULT_MODEL "gpt-4o-mini"
#define OTTO_DEFAULT_NAME "Openai"
#define OTTO_DEFAULT_BASEURL "https://api.openai.com"

#define OTTO_MAX_PROMPT_BYTES (1024U * 1024U)
#define OTTO_MAX_MODE_BYTES (256U * 1024U)
#define OTTO_MAX_RESPONSE_BYTES (16U * 1024U * 1024U)
#define OTTO_CURL_ERROR_SIZE 256

typedef enum {
    OTTO_OK = 0,
    OTTO_ERR_USAGE = 2,
    OTTO_ERR_CONFIG = 3,
    OTTO_ERR_NETWORK = 4,
    OTTO_ERR_API = 5,
    OTTO_ERR_MEMORY = 6
} OttoExitCode;

#endif
