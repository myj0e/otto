#include "otto/http.h"

#include <curl/curl.h>

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "otto/buffer.h"

typedef struct {
    OttoBuffer *buffer;
    size_t maximum_size;
} ResponseWriter;

static size_t response_write_callback(
    char *data,
    size_t size,
    size_t item_count,
    void *userdata
)
{
    ResponseWriter *writer = userdata;
    size_t amount;

    if (item_count != 0U && size > SIZE_MAX / item_count) {
        return 0U;
    }
    amount = size * item_count;

    if (amount > writer->maximum_size - writer->buffer->length) {
        return 0U;
    }

    if (otto_buffer_append_bytes(writer->buffer, data, amount) != 0) {
        return 0U;
    }
    return amount;
}

OttoExitCode otto_http_global_init(void)
{
    return curl_global_init(CURL_GLOBAL_DEFAULT) == CURLE_OK
        ? OTTO_OK
        : OTTO_ERR_NETWORK;
}

void otto_http_global_cleanup(void)
{
    curl_global_cleanup();
}

void otto_http_response_init(OttoHttpResponse *response)
{
    if (response == NULL) {
        return;
    }

    memset(response, 0, sizeof(*response));
}

void otto_http_response_free(OttoHttpResponse *response)
{
    if (response == NULL) {
        return;
    }

    free(response->body);
    otto_http_response_init(response);
}

static OttoExitCode curl_option_error(
    OttoHttpResponse *response,
    CURLcode code
)
{
    (void)snprintf(
        response->error_message,
        sizeof(response->error_message),
        "%s",
        curl_easy_strerror(code)
    );
    return OTTO_ERR_NETWORK;
}

OttoExitCode otto_http_chat(
    const OttoHttpRequest *request,
    OttoHttpResponse *response
)
{
    CURL *curl = NULL;
    struct curl_slist *headers = NULL;
    struct curl_slist *new_headers;
    OttoBuffer authorization;
    OttoBuffer body;
    ResponseWriter writer;
    char *authorization_header = NULL;
    CURLcode curl_code;
    CURLcode option_code;
    long http_status = 0L;
    size_t maximum_size;
    OttoExitCode result = OTTO_ERR_NETWORK;

    if (request == NULL || response == NULL || request->endpoint == NULL ||
        request->apikey == NULL || request->body == NULL) {
        return OTTO_ERR_USAGE;
    }

    otto_http_response_init(response);
    maximum_size = request->max_response_size == 0U
        ? OTTO_MAX_RESPONSE_BYTES
        : request->max_response_size;

    otto_buffer_init(&authorization);
    if (otto_buffer_append_cstr(&authorization, "Authorization: Bearer ") != 0 ||
        otto_buffer_append_cstr(&authorization, request->apikey) != 0) {
        otto_buffer_free(&authorization);
        return OTTO_ERR_MEMORY;
    }
    authorization_header = otto_buffer_take(&authorization);
    if (authorization_header == NULL) {
        otto_buffer_free(&authorization);
        return OTTO_ERR_MEMORY;
    }

    headers = curl_slist_append(headers, "Content-Type: application/json");
    if (headers == NULL) {
        free(authorization_header);
        return OTTO_ERR_MEMORY;
    }
    new_headers = curl_slist_append(headers, "Accept: application/json");
    if (new_headers == NULL) {
        curl_slist_free_all(headers);
        free(authorization_header);
        return OTTO_ERR_MEMORY;
    }
    headers = new_headers;
    new_headers = curl_slist_append(headers, authorization_header);
    if (new_headers == NULL) {
        curl_slist_free_all(headers);
        free(authorization_header);
        return OTTO_ERR_MEMORY;
    }
    headers = new_headers;
    free(authorization_header);
    authorization_header = NULL;

    curl = curl_easy_init();
    if (curl == NULL) {
        curl_slist_free_all(headers);
        return OTTO_ERR_NETWORK;
    }

    otto_buffer_init(&body);
    writer.buffer = &body;
    writer.maximum_size = maximum_size;

    option_code = curl_easy_setopt(curl, CURLOPT_URL, request->endpoint);
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(curl, CURLOPT_POST, 1L);
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(curl, CURLOPT_POSTFIELDS, request->body);
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(
        curl,
        CURLOPT_POSTFIELDSIZE_LARGE,
        (curl_off_t)request->body_length
    );
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers);
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, response_write_callback);
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(curl, CURLOPT_WRITEDATA, &writer);
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(curl, CURLOPT_ERRORBUFFER, response->error_message);
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(curl, CURLOPT_CONNECTTIMEOUT_MS, request->connect_timeout_ms);
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(curl, CURLOPT_TIMEOUT_MS, request->timeout_ms);
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(curl, CURLOPT_NOSIGNAL, 1L);
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, 0L);
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(curl, CURLOPT_SSL_VERIFYPEER, 1L);
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(curl, CURLOPT_SSL_VERIFYHOST, 2L);
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(curl, CURLOPT_ACCEPT_ENCODING, "");
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }
    option_code = curl_easy_setopt(curl, CURLOPT_USERAGENT, "otto/" OTTO_VERSION);
    if (option_code != CURLE_OK) {
        result = curl_option_error(response, option_code);
        goto cleanup;
    }

    curl_code = curl_easy_perform(curl);
    if (curl_code != CURLE_OK) {
        if (response->error_message[0] == '\0') {
            (void)snprintf(
                response->error_message,
                sizeof(response->error_message),
                "%s",
                curl_easy_strerror(curl_code)
            );
        }
        result = OTTO_ERR_NETWORK;
        goto cleanup;
    }

    curl_code = curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &http_status);
    if (curl_code != CURLE_OK) {
        result = curl_option_error(response, curl_code);
        goto cleanup;
    }

    response->http_status = http_status;
    response->body_length = body.length;
    response->body = otto_buffer_take(&body);
    if (response->body == NULL) {
        result = OTTO_ERR_MEMORY;
        goto cleanup;
    }

    result = OTTO_OK;

cleanup:
    otto_buffer_free(&body);
    curl_easy_cleanup(curl);
    curl_slist_free_all(headers);
    free(authorization_header);
    return result;
}
