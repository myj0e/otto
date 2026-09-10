#include "otto/input.h"

#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <termios.h>
#include <unistd.h>

#include "otto/util.h"

static int secret_terminal_fd = -1;
static struct termios secret_original_termios;

static void restore_secret_terminal(void)
{
    if (secret_terminal_fd >= 0) {
        (void)tcsetattr(
            secret_terminal_fd,
            TCSAFLUSH,
            &secret_original_termios
        );
        secret_terminal_fd = -1;
    }
}

static void secret_signal_handler(int signal_number)
{
    restore_secret_terminal();
    signal(signal_number, SIG_DFL);
    raise(signal_number);
}

static int install_secret_signal_handlers(
    struct sigaction *old_interrupt,
    struct sigaction *old_terminate
)
{
    struct sigaction action;

    memset(&action, 0, sizeof(action));
    sigemptyset(&action.sa_mask);
    action.sa_handler = secret_signal_handler;

    if (sigaction(SIGINT, &action, old_interrupt) != 0) {
        return -1;
    }
    if (sigaction(SIGTERM, &action, old_terminate) != 0) {
        (void)sigaction(SIGINT, old_interrupt, NULL);
        return -1;
    }

    return 0;
}

static void restore_secret_signal_handlers(
    const struct sigaction *old_interrupt,
    const struct sigaction *old_terminate
)
{
    (void)sigaction(SIGINT, old_interrupt, NULL);
    (void)sigaction(SIGTERM, old_terminate, NULL);
}

int otto_input_is_interactive(void)
{
    return isatty(STDIN_FILENO) && isatty(STDOUT_FILENO);
}

int otto_input_read_line(
    const char *prompt,
    const char *default_value,
    char **result
)
{
    char *line = NULL;
    char *trimmed = NULL;
    size_t capacity = 0;
    ssize_t length;

    if (prompt == NULL || result == NULL) {
        return -1;
    }

    *result = NULL;
    if (default_value != NULL && default_value[0] != '\0') {
        printf("%s [%s]: ", prompt, default_value);
    } else {
        printf("%s: ", prompt);
    }
    fflush(stdout);

    errno = 0;
    length = getline(&line, &capacity, stdin);
    if (length < 0) {
        free(line);
        return feof(stdin) ? 1 : -1;
    }

    trimmed = otto_trim_copy(line);
    free(line);
    if (trimmed == NULL) {
        return -1;
    }

    if (trimmed[0] == '\0' && default_value != NULL) {
        free(trimmed);
        trimmed = otto_strdup(default_value);
        if (trimmed == NULL) {
            return -1;
        }
    }

    *result = trimmed;
    return 0;
}

int otto_input_read_secret(const char *prompt, char **result)
{
    struct termios hidden_termios;
    struct sigaction old_interrupt;
    struct sigaction old_terminate;
    char *line = NULL;
    char *trimmed = NULL;
    size_t capacity = 0;
    ssize_t length;
    int signals_installed = 0;

    if (prompt == NULL || result == NULL || !isatty(STDIN_FILENO)) {
        return -1;
    }

    *result = NULL;
    if (tcgetattr(STDIN_FILENO, &secret_original_termios) != 0) {
        return -1;
    }

    hidden_termios = secret_original_termios;
    hidden_termios.c_lflag &= (tcflag_t)~ECHO;
    if (tcsetattr(STDIN_FILENO, TCSAFLUSH, &hidden_termios) != 0) {
        return -1;
    }

    secret_terminal_fd = STDIN_FILENO;
    if (install_secret_signal_handlers(&old_interrupt, &old_terminate) != 0) {
        restore_secret_terminal();
        return -1;
    }
    signals_installed = 1;

    printf("%s: ", prompt);
    fflush(stdout);
    length = getline(&line, &capacity, stdin);

    restore_secret_terminal();
    if (signals_installed) {
        restore_secret_signal_handlers(&old_interrupt, &old_terminate);
    }
    putchar('\n');

    if (length < 0) {
        free(line);
        return feof(stdin) ? 1 : -1;
    }

    trimmed = otto_trim_copy(line);
    free(line);
    if (trimmed == NULL) {
        return -1;
    }

    *result = trimmed;
    return 0;
}

int otto_input_confirm(const char *prompt, int default_yes)
{
    char *line = NULL;
    char *trimmed = NULL;
    size_t capacity = 0;
    ssize_t length;

    if (prompt == NULL) {
        return -1;
    }

    for (;;) {
        printf("%s [%s]: ", prompt, default_yes ? "Y/n" : "y/N");
        fflush(stdout);

        length = getline(&line, &capacity, stdin);
        if (length < 0) {
            free(line);
            return feof(stdin) ? 0 : -1;
        }

        trimmed = otto_trim_copy(line);
        if (trimmed == NULL) {
            free(line);
            return -1;
        }

        if (trimmed[0] == '\0') {
            free(trimmed);
            free(line);
            return default_yes ? 1 : 0;
        }

        if (strcasecmp(trimmed, "y") == 0 ||
            strcasecmp(trimmed, "yes") == 0) {
            free(trimmed);
            free(line);
            return 1;
        }

        if (strcasecmp(trimmed, "n") == 0 ||
            strcasecmp(trimmed, "no") == 0) {
            free(trimmed);
            free(line);
            return 0;
        }

        free(trimmed);
        puts("请输入 y 或 n。");
    }
}
