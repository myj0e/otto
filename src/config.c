#include "otto/config.h"

#include <errno.h>
#include <fcntl.h>
#include <pwd.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include "otto/input.h"
#include "otto/util.h"

static int path_join(const char *left, const char *right, char **result)
{
    size_t left_length;
    size_t right_length;
    int needs_separator;
    char *joined;

    if (left == NULL || right == NULL || result == NULL) {
        return -1;
    }

    left_length = strlen(left);
    right_length = strlen(right);
    needs_separator = left_length > 0U && left[left_length - 1U] != '/';

    {
        size_t extra = needs_separator ? 2U : 1U;
        if (left_length > SIZE_MAX - extra ||
            right_length > SIZE_MAX - extra - left_length) {
            return -1;
        }
        joined = malloc(left_length + right_length + extra);
    }
    if (joined == NULL) {
        return -1;
    }

    memcpy(joined, left, left_length);
    if (needs_separator) {
        joined[left_length] = '/';
        left_length++;
    }
    memcpy(joined + left_length, right, right_length + 1U);
    *result = joined;
    return 0;
}

static int path_dirname(const char *path, char **result)
{
    const char *slash;
    size_t length;
    char *directory;

    if (path == NULL || result == NULL || path[0] == '\0') {
        return -1;
    }

    slash = strrchr(path, '/');
    if (slash == NULL) {
        return otto_set_string(result, ".");
    }

    if (slash == path) {
        return otto_set_string(result, "/");
    }

    length = (size_t)(slash - path);
    directory = malloc(length + 1U);
    if (directory == NULL) {
        return -1;
    }

    memcpy(directory, path, length);
    directory[length] = '\0';
    free(*result);
    *result = directory;
    return 0;
}

static int directory_exists(const char *path)
{
    struct stat info;

    return stat(path, &info) == 0 && S_ISDIR(info.st_mode);
}

static int mkdir_p(const char *path)
{
    char *copy;
    char *cursor;
    size_t length;

    if (path == NULL || path[0] == '\0') {
        return -1;
    }

    if (directory_exists(path)) {
        return 0;
    }

    copy = otto_strdup(path);
    if (copy == NULL) {
        return -1;
    }

    length = strlen(copy);
    while (length > 1U && copy[length - 1U] == '/') {
        copy[--length] = '\0';
    }

    cursor = copy + (copy[0] == '/' ? 1 : 0);
    for (; *cursor != '\0'; cursor++) {
        if (*cursor != '/') {
            continue;
        }

        *cursor = '\0';
        if (copy[0] != '\0' && !directory_exists(copy)) {
            if (mkdir(copy, 0700) != 0 &&
                (errno != EEXIST || !directory_exists(copy))) {
                free(copy);
                return -1;
            }
        }
        *cursor = '/';
    }

    if (!directory_exists(copy)) {
        if (mkdir(copy, 0700) != 0 &&
            (errno != EEXIST || !directory_exists(copy))) {
            free(copy);
            return -1;
        }
    }

    free(copy);
    return 0;
}

static int set_message(char *message, size_t message_size, const char *text)
{
    if (message != NULL && message_size > 0U) {
        (void)snprintf(message, message_size, "%s", text);
    }
    return OTTO_ERR_CONFIG;
}

static int has_scheme(const char *value, const char *scheme)
{
    size_t length = strlen(scheme);
    return strncasecmp(value, scheme, length) == 0;
}

static int valid_baseurl(const char *baseurl)
{
    const char *host;
    size_t scheme_length;
    size_t index;

    if (baseurl == NULL) {
        return 0;
    }

    if (has_scheme(baseurl, "https://")) {
        scheme_length = 8U;
    } else if (has_scheme(baseurl, "http://")) {
        scheme_length = 7U;
    } else {
        return 0;
    }

    host = baseurl + scheme_length;
    if (*host == '\0' || *host == '/') {
        return 0;
    }

    if (strchr(baseurl, '?') != NULL || strchr(baseurl, '#') != NULL) {
        return 0;
    }

    for (index = 0U; baseurl[index] != '\0'; index++) {
        if (baseurl[index] == '\n' || baseurl[index] == '\r' ||
            baseurl[index] == ' ' || baseurl[index] == '\t') {
            return 0;
        }
    }

    return 1;
}

void otto_config_init(OttoConfig *config)
{
    if (config == NULL) {
        return;
    }

    memset(config, 0, sizeof(*config));
    config->model = otto_strdup(OTTO_DEFAULT_MODEL);
}

void otto_config_free(OttoConfig *config)
{
    if (config == NULL) {
        return;
    }

    free(config->name);
    free(config->baseurl);
    free(config->apikey);
    free(config->model);
    memset(config, 0, sizeof(*config));
}

OttoExitCode otto_config_get_directory(char **directory)
{
    const char *override;
    const char *config_home;
    const char *home;
    struct passwd *password_entry;
    char *base_directory = NULL;

    if (directory == NULL) {
        return OTTO_ERR_CONFIG;
    }
    *directory = NULL;

    override = getenv("OTTO_CONFIG");
    if (override != NULL && override[0] != '\0') {
        return path_dirname(override, directory) == 0
            ? OTTO_OK
            : OTTO_ERR_MEMORY;
    }

    config_home = getenv("XDG_CONFIG_HOME");
    if (config_home != NULL && config_home[0] != '\0') {
        if (path_join(config_home, "otto", directory) != 0) {
            return OTTO_ERR_MEMORY;
        }
    } else {
        home = getenv("HOME");
        if (home == NULL || home[0] == '\0') {
            password_entry = getpwuid(getuid());
            home = password_entry == NULL ? NULL : password_entry->pw_dir;
        }
        if (home == NULL || home[0] == '\0' ||
            path_join(home, ".config", &base_directory) != 0 ||
            path_join(base_directory, "otto", directory) != 0) {
            free(base_directory);
            return OTTO_ERR_CONFIG;
        }
    }

    free(base_directory);
    return OTTO_OK;
}

OttoExitCode otto_config_get_path(char **path)
{
    const char *override;
    char *directory = NULL;
    OttoExitCode code;

    if (path == NULL) {
        return OTTO_ERR_CONFIG;
    }
    *path = NULL;

    override = getenv("OTTO_CONFIG");
    if (override != NULL && override[0] != '\0') {
        *path = otto_strdup(override);
        return *path == NULL ? OTTO_ERR_MEMORY : OTTO_OK;
    }

    code = otto_config_get_directory(&directory);
    if (code != OTTO_OK) {
        return code;
    }

    if (path_join(directory, "config", path) != 0) {
        free(directory);
        return OTTO_ERR_MEMORY;
    }

    free(directory);
    return OTTO_OK;
}

OttoExitCode otto_config_load(
    const char *path,
    OttoConfig *config,
    int *found
)
{
    FILE *file;
    char *line = NULL;
    size_t capacity = 0U;
    ssize_t length;
    unsigned long line_number = 0U;

    if (path == NULL || config == NULL || found == NULL) {
        return OTTO_ERR_CONFIG;
    }

    *found = 0;
    file = fopen(path, "r");
    if (file == NULL) {
        if (errno == ENOENT) {
            return OTTO_OK;
        }
        fprintf(stderr, "otto: 无法读取配置文件 %s: %s\n", path, strerror(errno));
        return OTTO_ERR_CONFIG;
    }
    *found = 1;

    while ((length = getline(&line, &capacity, file)) >= 0) {
        char *entry;
        char *separator;
        char *key;
        char *value;

        line_number++;
        if (length > 0 && line[length - 1] == '\n') {
            line[length - 1] = '\0';
        }

        entry = otto_trim_copy(line);
        if (entry == NULL) {
            free(line);
            fclose(file);
            return OTTO_ERR_MEMORY;
        }

        if (entry[0] == '\0' || entry[0] == '#') {
            free(entry);
            continue;
        }

        separator = strchr(entry, '=');
        if (separator == NULL) {
            fprintf(
                stderr,
                "otto: 配置文件 %s 第 %lu 行格式错误\n",
                path,
                line_number
            );
            free(entry);
            free(line);
            fclose(file);
            return OTTO_ERR_CONFIG;
        }

        *separator = '\0';
        key = otto_trim_copy(entry);
        value = otto_trim_copy(separator + 1);
        free(entry);
        if (key == NULL || value == NULL) {
            free(key);
            free(value);
            free(line);
            fclose(file);
            return OTTO_ERR_MEMORY;
        }

        if (strcmp(key, "name") == 0) {
            if (otto_set_string(&config->name, value) != 0) {
                free(key);
                free(value);
                free(line);
                fclose(file);
                return OTTO_ERR_MEMORY;
            }
        } else if (strcmp(key, "baseurl") == 0) {
            if (otto_set_string(&config->baseurl, value) != 0) {
                free(key);
                free(value);
                free(line);
                fclose(file);
                return OTTO_ERR_MEMORY;
            }
        } else if (strcmp(key, "apikey") == 0) {
            if (otto_set_string(&config->apikey, value) != 0) {
                free(key);
                free(value);
                free(line);
                fclose(file);
                return OTTO_ERR_MEMORY;
            }
        } else if (strcmp(key, "model") == 0) {
            if (otto_set_string(&config->model, value) != 0) {
                free(key);
                free(value);
                free(line);
                fclose(file);
                return OTTO_ERR_MEMORY;
            }
        }

        free(key);
        free(value);
    }

    if (ferror(file)) {
        fprintf(stderr, "otto: 读取配置文件 %s 时发生错误\n", path);
        free(line);
        fclose(file);
        return OTTO_ERR_CONFIG;
    }

    free(line);
    fclose(file);
    return OTTO_OK;
}

OttoExitCode otto_config_validate(
    const OttoConfig *config,
    char *message,
    size_t message_size
)
{
    if (config == NULL) {
        return set_message(message, message_size, "配置为空");
    }
    if (config->name == NULL || config->name[0] == '\0') {
        return set_message(message, message_size, "name 不能为空");
    }
    if (config->baseurl == NULL || config->baseurl[0] == '\0' ||
        !valid_baseurl(config->baseurl)) {
        return set_message(message, message_size, "baseurl 不是有效的 HTTP/HTTPS 地址");
    }
    if (config->apikey == NULL || config->apikey[0] == '\0') {
        return set_message(message, message_size, "apikey 不能为空");
    }
    if (config->model == NULL || config->model[0] == '\0') {
        return set_message(message, message_size, "model 不能为空");
    }
    if (otto_contains_newline(config->name) ||
        otto_contains_newline(config->baseurl) ||
        otto_contains_newline(config->apikey) ||
        otto_contains_newline(config->model)) {
        return set_message(message, message_size, "配置项不能包含换行符");
    }

    if (message != NULL && message_size > 0U) {
        message[0] = '\0';
    }
    return OTTO_OK;
}

OttoExitCode otto_config_save_atomic(
    const char *path,
    const OttoConfig *config
)
{
    char validation_message[256];
    char *directory = NULL;
    char *temporary_path = NULL;
    size_t temporary_size;
    int file_descriptor = -1;
    FILE *file = NULL;
    int result;

    result = otto_config_validate(
        config,
        validation_message,
        sizeof(validation_message)
    );
    if (result != OTTO_OK) {
        fprintf(stderr, "otto: 配置无效：%s\n", validation_message);
        return (OttoExitCode)result;
    }

    if (path == NULL || path[0] == '\0' || path_dirname(path, &directory) != 0) {
        fprintf(stderr, "otto: 无法确定配置目录\n");
        free(directory);
        return OTTO_ERR_CONFIG;
    }

    if (mkdir_p(directory) != 0) {
        fprintf(stderr, "otto: 无法创建配置目录 %s: %s\n", directory, strerror(errno));
        free(directory);
        return OTTO_ERR_CONFIG;
    }
    free(directory);

    temporary_size = strlen(path) + sizeof(".tmp.XXXXXX");
    temporary_path = malloc(temporary_size);
    if (temporary_path == NULL) {
        return OTTO_ERR_MEMORY;
    }
    (void)snprintf(temporary_path, temporary_size, "%s.tmp.XXXXXX", path);

    file_descriptor = mkstemp(temporary_path);
    if (file_descriptor < 0) {
        fprintf(stderr, "otto: 无法创建临时配置文件: %s\n", strerror(errno));
        free(temporary_path);
        return OTTO_ERR_CONFIG;
    }

    if (fchmod(file_descriptor, 0600) != 0) {
        fprintf(stderr, "otto: 无法设置配置文件权限: %s\n", strerror(errno));
        close(file_descriptor);
        unlink(temporary_path);
        free(temporary_path);
        return OTTO_ERR_CONFIG;
    }

    file = fdopen(file_descriptor, "w");
    if (file == NULL) {
        fprintf(stderr, "otto: 无法打开临时配置文件: %s\n", strerror(errno));
        close(file_descriptor);
        unlink(temporary_path);
        free(temporary_path);
        return OTTO_ERR_CONFIG;
    }

    if (fprintf(
            file,
            "# OTTO (One-time.Talk once) configuration\n"
            "name=%s\n"
            "baseurl=%s\n"
            "apikey=%s\n"
            "model=%s\n",
            config->name,
            config->baseurl,
            config->apikey,
            config->model
        ) < 0 || fflush(file) != 0 || fsync(fileno(file)) != 0) {
        fprintf(stderr, "otto: 写入配置文件失败: %s\n", strerror(errno));
        fclose(file);
        unlink(temporary_path);
        free(temporary_path);
        return OTTO_ERR_CONFIG;
    }

    if (fclose(file) != 0) {
        fprintf(stderr, "otto: 关闭配置文件失败: %s\n", strerror(errno));
        unlink(temporary_path);
        free(temporary_path);
        return OTTO_ERR_CONFIG;
    }
    file = NULL;

    if (rename(temporary_path, path) != 0) {
        fprintf(stderr, "otto: 保存配置文件失败: %s\n", strerror(errno));
        unlink(temporary_path);
        free(temporary_path);
        return OTTO_ERR_CONFIG;
    }

    (void)chmod(path, 0600);
    free(temporary_path);
    return OTTO_OK;
}

OttoExitCode otto_config_save_values(
    const char *path,
    const char *name,
    const char *baseurl,
    const char *apikey,
    const char *model
)
{
    OttoConfig config;
    OttoExitCode code;

    otto_config_init(&config);
    if (otto_set_string(&config.name, name) != 0 ||
        otto_set_string(&config.baseurl, baseurl) != 0 ||
        otto_set_string(&config.apikey, apikey) != 0) {
        otto_config_free(&config);
        return OTTO_ERR_MEMORY;
    }
    if (model != NULL && otto_set_string(&config.model, model) != 0) {
        otto_config_free(&config);
        return OTTO_ERR_MEMORY;
    }

    code = otto_config_save_atomic(path, &config);
    otto_config_free(&config);
    return code;
}

static void print_masked_key(const char *apikey)
{
    size_t length;

    if (apikey == NULL || apikey[0] == '\0') {
        puts("  API Key: <empty>");
        return;
    }

    length = strlen(apikey);
    if (length <= 8U) {
        puts("  API Key: ********");
        return;
    }

    printf("  API Key: %.4s****%s\n", apikey, apikey + length - 4U);
}

OttoExitCode otto_config_interactive(const char *path)
{
    OttoConfig config;
    char *value = NULL;
    char validation_message[256];
    int found = 0;
    int input_result;
    int save_result;

    if (!otto_input_is_interactive()) {
        fprintf(stderr, "otto: --config 需要在交互式终端中运行\n");
        return OTTO_ERR_CONFIG;
    }

    otto_config_init(&config);
    input_result = otto_config_load(path, &config, &found);
    if (input_result != OTTO_OK) {
        otto_config_free(&config);
        return (OttoExitCode)input_result;
    }

    if (config.name == NULL && otto_set_string(&config.name, OTTO_DEFAULT_NAME) != 0) {
        otto_config_free(&config);
        return OTTO_ERR_MEMORY;
    }
    if (config.baseurl == NULL &&
        otto_set_string(&config.baseurl, OTTO_DEFAULT_BASEURL) != 0) {
        otto_config_free(&config);
        return OTTO_ERR_MEMORY;
    }

    puts("OTTO (One-time.Talk once) configuration");
    puts("输入新值，直接回车可保留当前默认值。");
    puts("");

    input_result = otto_input_read_line("Name", config.name, &value);
    if (input_result != 0) {
        fprintf(stderr, "otto: 配置已取消\n");
        free(value);
        otto_config_free(&config);
        return OTTO_ERR_CONFIG;
    }
    if (otto_set_string(&config.name, value) != 0) {
        free(value);
        otto_config_free(&config);
        return OTTO_ERR_MEMORY;
    }
    free(value);
    value = NULL;

    input_result = otto_input_read_line("Base URL", config.baseurl, &value);
    if (input_result != 0) {
        fprintf(stderr, "otto: 配置已取消\n");
        free(value);
        otto_config_free(&config);
        return OTTO_ERR_CONFIG;
    }
    if (otto_set_string(&config.baseurl, value) != 0) {
        free(value);
        otto_config_free(&config);
        return OTTO_ERR_MEMORY;
    }
    free(value);
    value = NULL;

    for (;;) {
        input_result = otto_input_read_secret("API Key", &value);
        if (input_result != 0) {
            fprintf(stderr, "otto: 配置已取消\n");
            free(value);
            otto_config_free(&config);
            return OTTO_ERR_CONFIG;
        }

        if (value[0] == '\0') {
            free(value);
            value = NULL;
            if (config.apikey != NULL && config.apikey[0] != '\0') {
                break;
            }
            puts("API Key 不能为空，请重新输入。");
            continue;
        }

        if (otto_set_string(&config.apikey, value) != 0) {
            free(value);
            otto_config_free(&config);
            return OTTO_ERR_MEMORY;
        }
        free(value);
        value = NULL;
        break;
    }

    input_result = otto_input_read_line("Model", config.model, &value);
    if (input_result != 0) {
        fprintf(stderr, "otto: 配置已取消\n");
        free(value);
        otto_config_free(&config);
        return OTTO_ERR_CONFIG;
    }
    if (otto_set_string(&config.model, value) != 0) {
        free(value);
        otto_config_free(&config);
        return OTTO_ERR_MEMORY;
    }
    free(value);

    input_result = otto_config_validate(
        &config,
        validation_message,
        sizeof(validation_message)
    );
    if (input_result != OTTO_OK) {
        fprintf(stderr, "otto: 配置无效：%s\n", validation_message);
        otto_config_free(&config);
        return (OttoExitCode)input_result;
    }

    puts("");
    puts("配置摘要：");
    printf("  Name: %s\n", config.name);
    printf("  Base URL: %s\n", config.baseurl);
    print_masked_key(config.apikey);
    printf("  Model: %s\n", config.model);

    save_result = otto_input_confirm("保存配置", 1);
    if (save_result < 0) {
        fprintf(stderr, "otto: 读取确认信息失败\n");
        otto_config_free(&config);
        return OTTO_ERR_CONFIG;
    }
    if (save_result == 0) {
        puts("配置未保存。");
        otto_config_free(&config);
        return OTTO_OK;
    }

    input_result = otto_config_save_atomic(path, &config);
    if (input_result == OTTO_OK) {
        printf("配置已保存到 %s\n", path);
    }
    otto_config_free(&config);
    (void)found;
    return (OttoExitCode)input_result;
}
