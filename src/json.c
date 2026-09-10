#include "otto/json.h"

#include <ctype.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "otto/buffer.h"
#include "otto/util.h"

enum {
    JSON_OK = 0,
    JSON_FOUND = 1,
    JSON_INVALID = -1,
    JSON_MEMORY = -2
};

typedef struct {
    const char *cursor;
    const char *end;
} JsonCursor;

static void json_skip_whitespace(JsonCursor *cursor)
{
    while (cursor->cursor < cursor->end &&
           isspace((unsigned char)*cursor->cursor)) {
        cursor->cursor++;
    }
}

static int json_hex_value(char value)
{
    if (value >= '0' && value <= '9') {
        return value - '0';
    }
    if (value >= 'a' && value <= 'f') {
        return value - 'a' + 10;
    }
    if (value >= 'A' && value <= 'F') {
        return value - 'A' + 10;
    }
    return -1;
}

static int json_read_hex4(JsonCursor *cursor, unsigned *value)
{
    unsigned result = 0U;
    int digit;
    size_t index;

    if ((size_t)(cursor->end - cursor->cursor) < 4U) {
        return JSON_INVALID;
    }

    for (index = 0U; index < 4U; index++) {
        digit = json_hex_value(cursor->cursor[index]);
        if (digit < 0) {
            return JSON_INVALID;
        }
        result = (result << 4U) | (unsigned)digit;
    }

    cursor->cursor += 4U;
    *value = result;
    return JSON_OK;
}

static int json_append_codepoint(OttoBuffer *buffer, unsigned codepoint)
{
    char encoded[4];
    size_t length;

    if (codepoint <= 0x7FU) {
        encoded[0] = (char)codepoint;
        length = 1U;
    } else if (codepoint <= 0x7FFU) {
        encoded[0] = (char)(0xC0U | (codepoint >> 6U));
        encoded[1] = (char)(0x80U | (codepoint & 0x3FU));
        length = 2U;
    } else if (codepoint <= 0xFFFFU) {
        encoded[0] = (char)(0xE0U | (codepoint >> 12U));
        encoded[1] = (char)(0x80U | ((codepoint >> 6U) & 0x3FU));
        encoded[2] = (char)(0x80U | (codepoint & 0x3FU));
        length = 3U;
    } else if (codepoint <= 0x10FFFFU) {
        encoded[0] = (char)(0xF0U | (codepoint >> 18U));
        encoded[1] = (char)(0x80U | ((codepoint >> 12U) & 0x3FU));
        encoded[2] = (char)(0x80U | ((codepoint >> 6U) & 0x3FU));
        encoded[3] = (char)(0x80U | (codepoint & 0x3FU));
        length = 4U;
    } else {
        return JSON_INVALID;
    }

    return otto_buffer_append_bytes(buffer, encoded, length) == 0
        ? JSON_OK
        : JSON_MEMORY;
}

static int json_skip_string(JsonCursor *cursor)
{
    char value;

    if (cursor->cursor >= cursor->end || *cursor->cursor != '"') {
        return JSON_INVALID;
    }
    cursor->cursor++;

    while (cursor->cursor < cursor->end) {
        value = *cursor->cursor++;
        if (value == '"') {
            return JSON_OK;
        }
        if ((unsigned char)value < 0x20U) {
            return JSON_INVALID;
        }
        if (value != '\\') {
            continue;
        }

        if (cursor->cursor >= cursor->end) {
            return JSON_INVALID;
        }
        value = *cursor->cursor++;
        if (value == 'u') {
            unsigned ignored;
            if (json_read_hex4(cursor, &ignored) != JSON_OK) {
                return JSON_INVALID;
            }
        } else if (value != '"' && value != '\\' && value != '/' &&
                   value != 'b' && value != 'f' && value != 'n' &&
                   value != 'r' && value != 't') {
            return JSON_INVALID;
        }
    }

    return JSON_INVALID;
}

static int json_parse_string(JsonCursor *cursor, char **result)
{
    OttoBuffer buffer;
    char value;
    unsigned codepoint;
    unsigned low_surrogate;
    const char *surrogate_start;
    int status;

    if (result == NULL || cursor->cursor >= cursor->end ||
        *cursor->cursor != '"') {
        return JSON_INVALID;
    }

    *result = NULL;
    otto_buffer_init(&buffer);
    cursor->cursor++;

    while (cursor->cursor < cursor->end) {
        value = *cursor->cursor++;
        if (value == '"') {
            *result = otto_buffer_take(&buffer);
            if (*result == NULL) {
                otto_buffer_free(&buffer);
                return JSON_MEMORY;
            }
            return JSON_OK;
        }
        if ((unsigned char)value < 0x20U) {
            otto_buffer_free(&buffer);
            return JSON_INVALID;
        }
        if (value != '\\') {
            if (otto_buffer_append_char(&buffer, value) != 0) {
                otto_buffer_free(&buffer);
                return JSON_MEMORY;
            }
            continue;
        }

        if (cursor->cursor >= cursor->end) {
            otto_buffer_free(&buffer);
            return JSON_INVALID;
        }

        value = *cursor->cursor++;
        switch (value) {
        case '"':
        case '\\':
        case '/':
            if (otto_buffer_append_char(&buffer, value) != 0) {
                otto_buffer_free(&buffer);
                return JSON_MEMORY;
            }
            break;
        case 'b':
            if (otto_buffer_append_char(&buffer, '\b') != 0) {
                otto_buffer_free(&buffer);
                return JSON_MEMORY;
            }
            break;
        case 'f':
            if (otto_buffer_append_char(&buffer, '\f') != 0) {
                otto_buffer_free(&buffer);
                return JSON_MEMORY;
            }
            break;
        case 'n':
            if (otto_buffer_append_char(&buffer, '\n') != 0) {
                otto_buffer_free(&buffer);
                return JSON_MEMORY;
            }
            break;
        case 'r':
            if (otto_buffer_append_char(&buffer, '\r') != 0) {
                otto_buffer_free(&buffer);
                return JSON_MEMORY;
            }
            break;
        case 't':
            if (otto_buffer_append_char(&buffer, '\t') != 0) {
                otto_buffer_free(&buffer);
                return JSON_MEMORY;
            }
            break;
        case 'u':
            status = json_read_hex4(cursor, &codepoint);
            if (status != JSON_OK) {
                otto_buffer_free(&buffer);
                return status;
            }

            if (codepoint >= 0xD800U && codepoint <= 0xDBFFU) {
                surrogate_start = cursor->cursor;
                if ((size_t)(cursor->end - cursor->cursor) >= 6U &&
                    cursor->cursor[0] == '\\' &&
                    cursor->cursor[1] == 'u') {
                    cursor->cursor += 2U;
                    if (json_read_hex4(cursor, &low_surrogate) == JSON_OK &&
                        low_surrogate >= 0xDC00U &&
                        low_surrogate <= 0xDFFFU) {
                        codepoint = 0x10000U +
                            ((codepoint - 0xD800U) << 10U) +
                            (low_surrogate - 0xDC00U);
                    } else {
                        cursor->cursor = surrogate_start;
                        codepoint = 0xFFFDU;
                    }
                } else {
                    codepoint = 0xFFFDU;
                }
            } else if (codepoint >= 0xDC00U && codepoint <= 0xDFFFU) {
                codepoint = 0xFFFDU;
            }

            status = json_append_codepoint(&buffer, codepoint);
            if (status != JSON_OK) {
                otto_buffer_free(&buffer);
                return status;
            }
            break;
        default:
            otto_buffer_free(&buffer);
            return JSON_INVALID;
        }
    }

    otto_buffer_free(&buffer);
    return JSON_INVALID;
}

static int json_skip_value(JsonCursor *cursor)
{
    char value;
    const char *primitive_start;

    json_skip_whitespace(cursor);
    if (cursor->cursor >= cursor->end) {
        return JSON_INVALID;
    }

    value = *cursor->cursor;
    if (value == '"') {
        return json_skip_string(cursor);
    }

    if (value == '{') {
        cursor->cursor++;
        json_skip_whitespace(cursor);
        if (cursor->cursor < cursor->end && *cursor->cursor == '}') {
            cursor->cursor++;
            return JSON_OK;
        }

        for (;;) {
            if (json_skip_string(cursor) != JSON_OK) {
                return JSON_INVALID;
            }
            json_skip_whitespace(cursor);
            if (cursor->cursor >= cursor->end || *cursor->cursor++ != ':') {
                return JSON_INVALID;
            }
            if (json_skip_value(cursor) != JSON_OK) {
                return JSON_INVALID;
            }
            json_skip_whitespace(cursor);
            if (cursor->cursor >= cursor->end) {
                return JSON_INVALID;
            }
            if (*cursor->cursor == '}') {
                cursor->cursor++;
                return JSON_OK;
            }
            if (*cursor->cursor++ != ',') {
                return JSON_INVALID;
            }
            json_skip_whitespace(cursor);
        }
    }

    if (value == '[') {
        cursor->cursor++;
        json_skip_whitespace(cursor);
        if (cursor->cursor < cursor->end && *cursor->cursor == ']') {
            cursor->cursor++;
            return JSON_OK;
        }

        for (;;) {
            if (json_skip_value(cursor) != JSON_OK) {
                return JSON_INVALID;
            }
            json_skip_whitespace(cursor);
            if (cursor->cursor >= cursor->end) {
                return JSON_INVALID;
            }
            if (*cursor->cursor == ']') {
                cursor->cursor++;
                return JSON_OK;
            }
            if (*cursor->cursor++ != ',') {
                return JSON_INVALID;
            }
            json_skip_whitespace(cursor);
        }
    }

    primitive_start = cursor->cursor;
    while (cursor->cursor < cursor->end &&
           *cursor->cursor != ',' && *cursor->cursor != ']' &&
           *cursor->cursor != '}' &&
           !isspace((unsigned char)*cursor->cursor)) {
        cursor->cursor++;
    }

    return cursor->cursor > primitive_start ? JSON_OK : JSON_INVALID;
}

static int json_parse_message_object(JsonCursor *cursor, char **content)
{
    char *key = NULL;
    int status;

    json_skip_whitespace(cursor);
    if (cursor->cursor >= cursor->end || *cursor->cursor++ != '{') {
        return JSON_INVALID;
    }
    json_skip_whitespace(cursor);
    if (cursor->cursor < cursor->end && *cursor->cursor == '}') {
        cursor->cursor++;
        return JSON_OK;
    }

    for (;;) {
        status = json_parse_string(cursor, &key);
        if (status != JSON_OK) {
            return status;
        }
        json_skip_whitespace(cursor);
        if (cursor->cursor >= cursor->end || *cursor->cursor++ != ':') {
            free(key);
            return JSON_INVALID;
        }
        json_skip_whitespace(cursor);

        if (strcmp(key, "content") == 0 && cursor->cursor < cursor->end &&
            *cursor->cursor == '"') {
            status = json_parse_string(cursor, content);
            free(key);
            return status == JSON_OK ? JSON_FOUND : status;
        }

        status = json_skip_value(cursor);
        free(key);
        key = NULL;
        if (status != JSON_OK) {
            return status;
        }

        json_skip_whitespace(cursor);
        if (cursor->cursor >= cursor->end) {
            return JSON_INVALID;
        }
        if (*cursor->cursor == '}') {
            cursor->cursor++;
            return JSON_OK;
        }
        if (*cursor->cursor++ != ',') {
            return JSON_INVALID;
        }
        json_skip_whitespace(cursor);
    }
}

static int json_parse_choice_object(JsonCursor *cursor, char **content)
{
    char *key = NULL;
    char *text = NULL;
    int status;

    json_skip_whitespace(cursor);
    if (cursor->cursor >= cursor->end || *cursor->cursor++ != '{') {
        return JSON_INVALID;
    }
    json_skip_whitespace(cursor);
    if (cursor->cursor < cursor->end && *cursor->cursor == '}') {
        cursor->cursor++;
        return JSON_OK;
    }

    for (;;) {
        status = json_parse_string(cursor, &key);
        if (status != JSON_OK) {
            free(text);
            return status;
        }
        json_skip_whitespace(cursor);
        if (cursor->cursor >= cursor->end || *cursor->cursor++ != ':') {
            free(key);
            free(text);
            return JSON_INVALID;
        }
        json_skip_whitespace(cursor);

        if (strcmp(key, "message") == 0 && cursor->cursor < cursor->end &&
            *cursor->cursor == '{') {
            status = json_parse_message_object(cursor, content);
            free(key);
            free(text);
            if (status == JSON_FOUND) {
                return JSON_FOUND;
            }
            if (status != JSON_OK) {
                return status;
            }
        } else if (strcmp(key, "text") == 0 && cursor->cursor < cursor->end &&
                   *cursor->cursor == '"') {
            status = json_parse_string(cursor, &text);
            free(key);
            key = NULL;
            if (status != JSON_OK) {
                free(text);
                return status;
            }
        } else {
            status = json_skip_value(cursor);
            free(key);
            key = NULL;
            if (status != JSON_OK) {
                free(text);
                return status;
            }
        }

        if (text != NULL) {
            *content = text;
            return JSON_FOUND;
        }

        json_skip_whitespace(cursor);
        if (cursor->cursor >= cursor->end) {
            return JSON_INVALID;
        }
        if (*cursor->cursor == '}') {
            cursor->cursor++;
            return JSON_OK;
        }
        if (*cursor->cursor++ != ',') {
            return JSON_INVALID;
        }
        json_skip_whitespace(cursor);
    }
}

static int json_parse_choices(JsonCursor *cursor, char **content)
{
    int status;

    json_skip_whitespace(cursor);
    if (cursor->cursor >= cursor->end || *cursor->cursor++ != '[') {
        return JSON_INVALID;
    }
    json_skip_whitespace(cursor);
    if (cursor->cursor < cursor->end && *cursor->cursor == ']') {
        cursor->cursor++;
        return JSON_OK;
    }

    for (;;) {
        json_skip_whitespace(cursor);
        if (cursor->cursor >= cursor->end) {
            return JSON_INVALID;
        }

        if (*cursor->cursor == '{') {
            status = json_parse_choice_object(cursor, content);
        } else {
            status = json_skip_value(cursor);
        }
        if (status == JSON_FOUND) {
            return JSON_FOUND;
        }
        if (status != JSON_OK) {
            return status;
        }

        json_skip_whitespace(cursor);
        if (cursor->cursor >= cursor->end) {
            return JSON_INVALID;
        }
        if (*cursor->cursor == ']') {
            cursor->cursor++;
            return JSON_OK;
        }
        if (*cursor->cursor++ != ',') {
            return JSON_INVALID;
        }
    }
}

static int json_parse_error_object(
    JsonCursor *cursor,
    OttoJsonResult *result
)
{
    char *key = NULL;
    char *value = NULL;
    int status;

    json_skip_whitespace(cursor);
    if (cursor->cursor >= cursor->end || *cursor->cursor++ != '{') {
        return JSON_INVALID;
    }
    json_skip_whitespace(cursor);
    if (cursor->cursor < cursor->end && *cursor->cursor == '}') {
        cursor->cursor++;
        return JSON_OK;
    }

    for (;;) {
        status = json_parse_string(cursor, &key);
        if (status != JSON_OK) {
            return status;
        }
        json_skip_whitespace(cursor);
        if (cursor->cursor >= cursor->end || *cursor->cursor++ != ':') {
            free(key);
            return JSON_INVALID;
        }
        json_skip_whitespace(cursor);

        if ((strcmp(key, "message") == 0 || strcmp(key, "type") == 0) &&
            cursor->cursor < cursor->end && *cursor->cursor == '"') {
            status = json_parse_string(cursor, &value);
            if (status != JSON_OK) {
                free(key);
                return status;
            }
            if (strcmp(key, "message") == 0) {
                free(result->error_message);
                result->error_message = value;
                value = NULL;
            } else {
                free(result->error_type);
                result->error_type = value;
                value = NULL;
            }
        } else {
            status = json_skip_value(cursor);
            if (status != JSON_OK) {
                free(key);
                return status;
            }
        }
        free(value);
        value = NULL;
        free(key);
        key = NULL;

        json_skip_whitespace(cursor);
        if (cursor->cursor >= cursor->end) {
            return JSON_INVALID;
        }
        if (*cursor->cursor == '}') {
            cursor->cursor++;
            return JSON_OK;
        }
        if (*cursor->cursor++ != ',') {
            return JSON_INVALID;
        }
    }
}

static int set_result_message(char **target, const char *value)
{
    return otto_set_string(target, value) == 0 ? JSON_OK : JSON_MEMORY;
}

static int append_json_string(OttoBuffer *buffer, const char *value)
{
    const unsigned char *cursor = (const unsigned char *)value;
    char escaped[7];
    int length;

    if (otto_buffer_append_char(buffer, '"') != 0) {
        return JSON_MEMORY;
    }

    while (*cursor != '\0') {
        switch (*cursor) {
        case '"':
            if (otto_buffer_append_cstr(buffer, "\\\"") != 0) {
                return JSON_MEMORY;
            }
            break;
        case '\\':
            if (otto_buffer_append_cstr(buffer, "\\\\") != 0) {
                return JSON_MEMORY;
            }
            break;
        case '\b':
            if (otto_buffer_append_cstr(buffer, "\\b") != 0) {
                return JSON_MEMORY;
            }
            break;
        case '\f':
            if (otto_buffer_append_cstr(buffer, "\\f") != 0) {
                return JSON_MEMORY;
            }
            break;
        case '\n':
            if (otto_buffer_append_cstr(buffer, "\\n") != 0) {
                return JSON_MEMORY;
            }
            break;
        case '\r':
            if (otto_buffer_append_cstr(buffer, "\\r") != 0) {
                return JSON_MEMORY;
            }
            break;
        case '\t':
            if (otto_buffer_append_cstr(buffer, "\\t") != 0) {
                return JSON_MEMORY;
            }
            break;
        default:
            if (*cursor < 0x20U) {
                length = snprintf(
                    escaped,
                    sizeof(escaped),
                    "\\u%04x",
                    (unsigned)*cursor
                );
                if (length < 0 ||
                    otto_buffer_append_bytes(buffer, escaped, (size_t)length) != 0) {
                    return JSON_MEMORY;
                }
            } else if (otto_buffer_append_char(buffer, (char)*cursor) != 0) {
                return JSON_MEMORY;
            }
            break;
        }
        cursor++;
    }

    return otto_buffer_append_char(buffer, '"') == 0
        ? JSON_OK
        : JSON_MEMORY;
}

void otto_json_result_free(OttoJsonResult *result)
{
    if (result == NULL) {
        return;
    }

    free(result->content);
    free(result->error_message);
    free(result->error_type);
    memset(result, 0, sizeof(*result));
}

OttoExitCode otto_json_build_request(
    const char *model,
    const char *system_prompt,
    const char *prompt,
    char **json,
    size_t *json_length
)
{
    OttoBuffer buffer;
    int status;

    if (model == NULL || prompt == NULL || json == NULL || json_length == NULL) {
        return OTTO_ERR_USAGE;
    }

    *json = NULL;
    *json_length = 0U;
    otto_buffer_init(&buffer);

    if (otto_buffer_append_cstr(&buffer, "{\"model\":") != 0) {
        otto_buffer_free(&buffer);
        return OTTO_ERR_MEMORY;
    }
    status = append_json_string(&buffer, model);
    if (status != JSON_OK || otto_buffer_append_cstr(&buffer, ",\"messages\":[") != 0) {
        otto_buffer_free(&buffer);
        return OTTO_ERR_MEMORY;
    }

    if (system_prompt != NULL) {
        if (otto_buffer_append_cstr(
                &buffer,
                "{\"role\":\"system\",\"content\":"
            ) != 0 ||
            append_json_string(&buffer, system_prompt) != JSON_OK ||
            otto_buffer_append_cstr(&buffer, "},{") != 0) {
            otto_buffer_free(&buffer);
            return OTTO_ERR_MEMORY;
        }
    } else if (otto_buffer_append_cstr(&buffer, "{") != 0) {
        otto_buffer_free(&buffer);
        return OTTO_ERR_MEMORY;
    }

    if (otto_buffer_append_cstr(
            &buffer,
            "\"role\":\"user\",\"content\":"
        ) != 0) {
        otto_buffer_free(&buffer);
        return OTTO_ERR_MEMORY;
    }
    status = append_json_string(&buffer, prompt);
    if (status != JSON_OK ||
        otto_buffer_append_cstr(&buffer, "}],\"stream\":false}") != 0) {
        otto_buffer_free(&buffer);
        return OTTO_ERR_MEMORY;
    }

    *json_length = buffer.length;
    *json = otto_buffer_take(&buffer);
    if (*json == NULL) {
        otto_buffer_free(&buffer);
        return OTTO_ERR_MEMORY;
    }
    return OTTO_OK;
}

OttoExitCode otto_json_parse_response(
    const char *json,
    size_t length,
    OttoJsonResult *result
)
{
    JsonCursor cursor;
    char *key = NULL;
    int status;

    if (json == NULL || result == NULL) {
        return OTTO_ERR_API;
    }

    memset(result, 0, sizeof(*result));
    cursor.cursor = json;
    cursor.end = json + length;
    json_skip_whitespace(&cursor);

    if (cursor.cursor >= cursor.end || *cursor.cursor++ != '{') {
        (void)set_result_message(&result->error_message, "API 响应不是有效的 JSON 对象");
        return OTTO_ERR_API;
    }

    json_skip_whitespace(&cursor);
    if (cursor.cursor < cursor.end && *cursor.cursor == '}') {
        cursor.cursor++;
        (void)set_result_message(&result->error_message, "API 响应中没有回答内容");
        return OTTO_ERR_API;
    }

    for (;;) {
        status = json_parse_string(&cursor, &key);
        if (status != JSON_OK) {
            free(key);
            (void)set_result_message(&result->error_message, "API 响应不是有效的 JSON");
            return status == JSON_MEMORY ? OTTO_ERR_MEMORY : OTTO_ERR_API;
        }
        json_skip_whitespace(&cursor);
        if (cursor.cursor >= cursor.end || *cursor.cursor++ != ':') {
            free(key);
            (void)set_result_message(&result->error_message, "API 响应不是有效的 JSON");
            return OTTO_ERR_API;
        }
        json_skip_whitespace(&cursor);

        if (strcmp(key, "choices") == 0) {
            status = json_parse_choices(&cursor, &result->content);
            free(key);
            key = NULL;
            if (status == JSON_FOUND) {
                return OTTO_OK;
            }
            if (status != JSON_OK) {
                (void)set_result_message(&result->error_message, "choices 字段格式错误");
                return status == JSON_MEMORY ? OTTO_ERR_MEMORY : OTTO_ERR_API;
            }
        } else if (strcmp(key, "error") == 0 && cursor.cursor < cursor.end &&
                   *cursor.cursor == '{') {
            status = json_parse_error_object(&cursor, result);
            free(key);
            key = NULL;
            if (status != JSON_OK) {
                (void)set_result_message(&result->error_message, "error 字段格式错误");
                return status == JSON_MEMORY ? OTTO_ERR_MEMORY : OTTO_ERR_API;
            }
        } else {
            status = json_skip_value(&cursor);
            free(key);
            key = NULL;
            if (status != JSON_OK) {
                (void)set_result_message(&result->error_message, "API 响应不是有效的 JSON");
                return status == JSON_MEMORY ? OTTO_ERR_MEMORY : OTTO_ERR_API;
            }
        }

        json_skip_whitespace(&cursor);
        if (cursor.cursor >= cursor.end) {
            break;
        }
        if (*cursor.cursor == '}') {
            cursor.cursor++;
            break;
        }
        if (*cursor.cursor++ != ',') {
            (void)set_result_message(&result->error_message, "API 响应不是有效的 JSON");
            return OTTO_ERR_API;
        }
        json_skip_whitespace(&cursor);
    }

    if (result->error_message == NULL) {
        (void)set_result_message(&result->error_message, "API 响应中没有回答内容");
    }
    return OTTO_ERR_API;
}
