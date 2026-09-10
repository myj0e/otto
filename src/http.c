#include "otto/http.h"

#include <curl/curl.h>

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "otto/buffer.h"
#include "otto/json.h"

typedef struct {
    OttoBuffer *buffer;
    size_t maximum_size;
    int stream;
    OttoHttpStreamCallback stream_callback;
    void *stream_userdata;
    OttoBuffer line;
    OttoBuffer event_data;
    int pending_carriage_return;
    OttoExitCode stream_code;
    char stream_error_message[OTTO_CURL_ERROR_SIZE];
    int streamed;
    int stream_ends_with_newline;
} ResponseWriter;

static void reset_buffer(OttoBuffer *buffer)
{
    if (buffer->data != NULL) {
        buffer->data[0] = '\0';
    }
    buffer->length = 0U;
}

static int set_stream_error(
    ResponseWriter *writer,
    OttoExitCode code,
    const char *message
)
{
    if (writer->stream_code == OTTO_OK) {
        writer->stream_code = code;
        (void)snprintf(
            writer->stream_error_message,
            sizeof(writer->stream_error_message),
            "%s",
            message
        );
    }
    return -1;
}

static int append_limited(
    OttoBuffer *buffer,
    const char *data,
    size_t length,
    size_t maximum_size,
    ResponseWriter *writer,
    OttoExitCode error_code,
    const char *error_message
)
{
    if (buffer->length > maximum_size || length > maximum_size - buffer->length) {
        return set_stream_error(writer, error_code, error_message);
    }
    if (otto_buffer_append_bytes(buffer, data, length) != 0) {
        return set_stream_error(writer, OTTO_ERR_MEMORY, "处理 SSE 数据时内存不足");
    }
    return 0;
}

static int dispatch_sse_event(ResponseWriter *writer)
{
    char *content = NULL;
    size_t content_length;
    OttoExitCode code;

    if (writer->event_data.length == 0U) {
        return 0;
    }

    writer->streamed = 1;
    if (strcmp(writer->event_data.data, "[DONE]") == 0) {
        reset_buffer(&writer->event_data);
        return 0;
    }

    code = otto_json_parse_stream_event(
        writer->event_data.data,
        writer->event_data.length,
        &content
    );
    if (code != OTTO_OK) {
        free(content);
        return set_stream_error(
            writer,
            code,
            code == OTTO_ERR_MEMORY
                ? "处理 SSE 数据时内存不足"
                : "SSE 数据不是有效的 Chat Completions JSON"
        );
    }

    if (content != NULL) {
        content_length = strlen(content);
        if (content_length > 0U && writer->stream_callback != NULL &&
            writer->stream_callback(
                content,
                content_length,
                writer->stream_userdata
            ) != 0) {
            free(content);
            return set_stream_error(
                writer,
                OTTO_ERR_API,
                "写入流式回答失败"
            );
        }
        if (content_length > 0U) {
            writer->stream_ends_with_newline =
                content[content_length - 1U] == '\n';
        }
    }
    free(content);
    reset_buffer(&writer->event_data);
    return 0;
}

static int process_sse_line(ResponseWriter *writer)
{
    const char *value;
    size_t value_length;

    if (writer->line.length == 0U) {
        return dispatch_sse_event(writer);
    }

    if (writer->line.data[0] == ':') {
        reset_buffer(&writer->line);
        return 0;
    }

    if (writer->line.length >= 5U &&
        memcmp(writer->line.data, "data:", 5U) == 0) {
        value = writer->line.data + 5U;
        value_length = writer->line.length - 5U;
        if (value_length > 0U && value[0] == ' ') {
            value++;
            value_length--;
        }

        if (writer->event_data.length > 0U &&
            append_limited(
                &writer->event_data,
                "\n",
                1U,
                writer->maximum_size,
                writer,
                OTTO_ERR_API,
                "SSE 事件超过响应大小限制"
            ) != 0) {
            return -1;
        }
        if (append_limited(
                &writer->event_data,
                value,
                value_length,
                writer->maximum_size,
                writer,
                OTTO_ERR_API,
                "SSE 事件超过响应大小限制"
            ) != 0) {
            return -1;
        }
    }

    reset_buffer(&writer->line);
    return 0;
}

static int feed_sse(
    ResponseWriter *writer,
    const char *data,
    size_t length
)
{
    size_t index;
    unsigned char value;

    for (index = 0U; index < length; index++) {
        value = (unsigned char)data[index];
        if (writer->pending_carriage_return) {
            if (value == '\n') {
                writer->pending_carriage_return = 0;
                if (process_sse_line(writer) != 0) {
                    return -1;
                }
                continue;
            }
            writer->pending_carriage_return = 0;
            if (process_sse_line(writer) != 0) {
                return -1;
            }
        }

        if (value == '\r') {
            writer->pending_carriage_return = 1;
        } else if (value == '\n') {
            if (process_sse_line(writer) != 0) {
                return -1;
            }
        } else if (append_limited(
                       &writer->line,
                       data + index,
                       1U,
                       writer->maximum_size,
                       writer,
                       OTTO_ERR_API,
                       "SSE 行超过响应大小限制"
                   ) != 0) {
            return -1;
        }
    }
    return 0;
}

static int finish_sse(ResponseWriter *writer)
{
    if (writer->pending_carriage_return) {
        writer->pending_carriage_return = 0;
        if (process_sse_line(writer) != 0) {
            return -1;
        }
    } else if (writer->line.length > 0U && process_sse_line(writer) != 0) {
        return -1;
    }

    if (writer->event_data.length > 0U && dispatch_sse_event(writer) != 0) {
        return -1;
    }
    return 0;
}

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

    if (writer->buffer->length > writer->maximum_size ||
        amount > writer->maximum_size - writer->buffer->length) {
        return 0U;
    }

    if (otto_buffer_append_bytes(writer->buffer, data, amount) != 0) {
        return 0U;
    }
    if (writer->stream && feed_sse(writer, data, amount) != 0) {
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
    new_headers = curl_slist_append(
        headers,
        request->stream ? "Accept: text/event-stream" : "Accept: application/json"
    );
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

    memset(&writer, 0, sizeof(writer));
    otto_buffer_init(&body);
    otto_buffer_init(&writer.line);
    otto_buffer_init(&writer.event_data);
    writer.buffer = &body;
    writer.maximum_size = maximum_size;
    writer.stream = request->stream != 0;
    writer.stream_callback = request->stream_callback;
    writer.stream_userdata = request->stream_userdata;
    writer.stream_code = OTTO_OK;

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
    if (curl_code == CURLE_OK && writer.stream && finish_sse(&writer) != 0) {
        (void)snprintf(
            response->error_message,
            sizeof(response->error_message),
            "%s",
            writer.stream_error_message
        );
        result = writer.stream_code;
        goto cleanup;
    }
    if (writer.stream_code != OTTO_OK) {
        (void)snprintf(
            response->error_message,
            sizeof(response->error_message),
            "%s",
            writer.stream_error_message
        );
        result = writer.stream_code;
        goto cleanup;
    }
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
    response->streamed = writer.streamed;
    response->stream_ends_with_newline = writer.stream_ends_with_newline;
    response->body_length = body.length;
    response->body = otto_buffer_take(&body);
    if (response->body == NULL) {
        result = OTTO_ERR_MEMORY;
        goto cleanup;
    }

    result = OTTO_OK;

cleanup:
    otto_buffer_free(&body);
    otto_buffer_free(&writer.line);
    otto_buffer_free(&writer.event_data);
    curl_easy_cleanup(curl);
    curl_slist_free_all(headers);
    free(authorization_header);
    return result;
}
