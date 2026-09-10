#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "otto/cli.h"
#include "otto/config.h"
#include "otto/http.h"
#include "otto/json.h"
#include "otto/mode.h"
#include "otto/url.h"

static OttoExitCode handle_config_command(const OttoCliOptions *options)
{
    char *path = NULL;
    OttoExitCode code;

    code = otto_config_get_path(&path);
    if (code != OTTO_OK) {
        fprintf(stderr, "otto: 无法确定配置文件路径\n");
        return code;
    }

    if (!options->has_config_options) {
        code = otto_config_interactive(path);
        free(path);
        return code;
    }

    if (options->name == NULL || options->baseurl == NULL ||
        options->apikey == NULL) {
        fprintf(
            stderr,
            "otto: 参数式配置必须同时提供 --name、--baseurl 和 --apikey\n"
        );
        free(path);
        return OTTO_ERR_USAGE;
    }

    code = otto_config_save_values(
        path,
        options->name,
        options->baseurl,
        options->apikey,
        options->model
    );
    if (code == OTTO_OK) {
        printf("配置已保存到 %s\n", path);
    }
    free(path);
    return code;
}

static OttoExitCode handle_mode_command(const OttoCliOptions *options)
{
    char *system_prompt = NULL;
    int found = 0;
    OttoExitCode code;

    if (options->mode == NULL) {
        code = otto_mode_set_active(NULL);
        if (code == OTTO_OK) {
            puts("已清除当前模式，之后的 otto <问题> 将使用裸生成模式。");
        }
        return code;
    }

    code = otto_mode_load(options->mode, &system_prompt, &found);
    if (code == OTTO_OK && !found) {
        fprintf(stderr, "otto: 指定模式没有可用的提示词文件\n");
        code = OTTO_ERR_CONFIG;
    }
    if (code == OTTO_OK) {
        code = otto_mode_set_active(options->mode);
    }
    if (code == OTTO_OK) {
        printf("已切换到模式：%s\n", options->mode);
    }

    free(system_prompt);
    return code;
}

static OttoExitCode ask_question(
    int argc,
    char **argv,
    const OttoCliOptions *options
)
{
    OttoConfig config;
    OttoHttpRequest request;
    OttoHttpResponse response;
    OttoJsonResult json_result;
    char validation_message[256];
    char *config_path = NULL;
    char *prompt = NULL;
    char *endpoint = NULL;
    char *body = NULL;
    char *system_prompt = NULL;
    char *active_mode = NULL;
    size_t body_length = 0U;
    int found = 0;
    int active_mode_found = 0;
    int mode_found = 0;
    const char *selected_mode = NULL;
    OttoExitCode code;

    prompt = otto_cli_join_prompt(argc, argv, options->prompt_arg_start);
    if (prompt == NULL) {
        fprintf(stderr, "otto: 无法生成问题内容，可能是内存不足或内容过长\n");
        return OTTO_ERR_USAGE;
    }
    if (prompt[0] == '\0') {
        fprintf(stderr, "otto: 问题内容不能为空\n");
        free(prompt);
        return OTTO_ERR_USAGE;
    }

    otto_config_init(&config);
    otto_http_response_init(&response);
    memset(&json_result, 0, sizeof(json_result));

    code = otto_config_get_path(&config_path);
    if (code != OTTO_OK) {
        fprintf(stderr, "otto: 无法确定配置文件路径\n");
        goto cleanup;
    }

    code = otto_config_load(config_path, &config, &found);
    if (code != OTTO_OK) {
        goto cleanup;
    }
    if (!found) {
        fprintf(stderr, "otto: 尚未配置 API，请先运行 otto --config\n");
        code = OTTO_ERR_CONFIG;
        goto cleanup;
    }

    code = otto_config_validate(
        &config,
        validation_message,
        sizeof(validation_message)
    );
    if (code != OTTO_OK) {
        fprintf(stderr, "otto: 配置无效：%s\n", validation_message);
        goto cleanup;
    }

    if (!options->raw_mode) {
        selected_mode = options->mode;
        if (selected_mode == NULL) {
            code = otto_mode_get_active(&active_mode, &active_mode_found);
            if (code != OTTO_OK) {
                goto cleanup;
            }
            if (active_mode_found) {
                selected_mode = active_mode;
            }
        }

        if (selected_mode == NULL) {
            goto build_request;
        }

        code = otto_mode_load(
            selected_mode,
            &system_prompt,
            &mode_found
        );
        if (code != OTTO_OK) {
            goto cleanup;
        }
        if (!mode_found) {
            fprintf(stderr, "otto: 指定模式没有可用的提示词文件\n");
            code = OTTO_ERR_CONFIG;
            goto cleanup;
        }
    }

build_request:
    code = otto_build_endpoint(config.baseurl, &endpoint);
    if (code != OTTO_OK) {
        goto cleanup;
    }

    code = otto_json_build_request(
        config.model,
        system_prompt,
        prompt,
        &body,
        &body_length
    );
    if (code != OTTO_OK) {
        fprintf(stderr, "otto: 无法生成请求内容\n");
        goto cleanup;
    }

    code = otto_http_global_init();
    if (code != OTTO_OK) {
        fprintf(stderr, "otto: libcurl 初始化失败\n");
        goto cleanup;
    }

    memset(&request, 0, sizeof(request));
    request.endpoint = endpoint;
    request.apikey = config.apikey;
    request.body = body;
    request.body_length = body_length;
    request.connect_timeout_ms = 15000L;
    request.timeout_ms = 120000L;
    request.max_response_size = OTTO_MAX_RESPONSE_BYTES;

    code = otto_http_chat(&request, &response);
    otto_http_global_cleanup();
    if (code != OTTO_OK) {
        fprintf(
            stderr,
            "otto: %s：%s\n",
            code == OTTO_ERR_MEMORY ? "处理请求时内存不足" : "网络请求失败",
            response.error_message[0] == '\0'
                ? "未知错误"
                : response.error_message
        );
        goto cleanup;
    }

    code = otto_json_parse_response(
        response.body,
        response.body_length,
        &json_result
    );
    if (response.http_status < 200L || response.http_status >= 300L) {
        if (json_result.error_message != NULL) {
            fprintf(
                stderr,
                "otto: API 请求失败 (HTTP %ld)：%s\n",
                response.http_status,
                json_result.error_message
            );
        } else {
            fprintf(
                stderr,
                "otto: API 请求失败 (HTTP %ld)\n",
                response.http_status
            );
        }
        code = OTTO_ERR_API;
        goto cleanup;
    }

    if (code != OTTO_OK) {
        fprintf(
            stderr,
            "otto: 无法解析 API 响应：%s\n",
            json_result.error_message == NULL
                ? "未知错误"
                : json_result.error_message
        );
        goto cleanup;
    }

    {
        size_t content_length = strlen(json_result.content);
        int needs_newline = content_length == 0U ||
            json_result.content[content_length - 1U] != '\n';

        if (fwrite(
                json_result.content,
                1U,
                content_length,
                stdout
            ) != content_length ||
            (needs_newline && fputc('\n', stdout) == EOF)) {
        fprintf(stderr, "otto: 写入回答失败\n");
        code = OTTO_ERR_API;
        goto cleanup;
        }
    }

    code = OTTO_OK;

cleanup:
    free(config_path);
    free(prompt);
    free(endpoint);
    free(body);
    free(system_prompt);
    free(active_mode);
    otto_json_result_free(&json_result);
    otto_http_response_free(&response);
    otto_config_free(&config);
    return code;
}

int main(int argc, char **argv)
{
    OttoCliOptions options;
    OttoExitCode code;

    code = otto_cli_parse(argc, argv, &options);
    if (code != OTTO_OK) {
        otto_cli_free(&options);
        return code;
    }

    switch (options.command) {
    case OTTO_COMMAND_HELP:
        otto_cli_print_help();
        code = OTTO_OK;
        break;
    case OTTO_COMMAND_VERSION:
        otto_cli_print_version();
        code = OTTO_OK;
        break;
    case OTTO_COMMAND_CONFIG:
        code = handle_config_command(&options);
        break;
    case OTTO_COMMAND_MODE:
        code = handle_mode_command(&options);
        break;
    case OTTO_COMMAND_ASK:
    default:
        code = ask_question(argc, argv, &options);
        break;
    }

    otto_cli_free(&options);
    return code;
}
