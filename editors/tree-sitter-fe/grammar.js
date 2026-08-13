/**
 * Tree-sitter grammar for the flight-engineer procedure language.
 *
 * It mirrors `docs/language.md`, with one deliberate difference: this grammar
 * is a little more permissive than `fe-lang`'s parser, because an editor has to
 * keep highlighting a file that is halfway through being typed. Anything this
 * grammar accepts that the compiler rejects is reported by the compiler, which
 * remains the only authority on whether a procedure is legal.
 *
 * Keywords are contextual, exactly as they are in the real lexer: `name`,
 * `check`, `set` and the rest are keywords only where a keyword is expected,
 * so `hydraulic.name.check` stays a legal path. That falls out of tree-sitter's
 * keyword extraction (`word: $ => $.identifier`) rather than being spelled out.
 */

module.exports = grammar({
  name: 'fe',

  word: $ => $.identifier,

  extras: $ => [/\s/, $.line_comment, $.block_comment],

  rules: {
    source_file: $ => repeat($.procedure),

    procedure: $ =>
      seq(
        'procedure',
        field('name', $.identifier),
        '{',
        repeat($.metadata),
        repeat($.step),
        '}',
      ),

    // ---------------------------------------------------------------- metadata

    metadata: $ =>
      choice(
        $.name_entry,
        $.description_entry,
        $.category_entry,
        $.priority_entry,
        $.revision_entry,
        $.trigger_entry,
        $.require_entry,
      ),

    name_entry: $ => seq('name', field('value', $.string)),
    description_entry: $ => seq('description', field('value', $.string)),
    category_entry: $ => seq('category', field('value', $.identifier)),
    priority_entry: $ => seq('priority', field('value', $.number)),
    revision_entry: $ => seq('revision', field('value', $.number)),
    trigger_entry: $ => seq('trigger', field('condition', $._expression)),
    require_entry: $ =>
      seq('require', field('condition', $._expression), field('message', optional($.string))),

    // ------------------------------------------------------------------- steps

    step: $ =>
      choice(
        $.check_step,
        $.set_step,
        $.verb_step,
        $.notify_step,
        $.call_step,
        $.wait_step,
        $.if_step,
        $.complete_step,
        $.fail_step,
      ),

    check_step: $ => seq('check', field('control', $.control_path)),

    set_step: $ =>
      seq(
        'set',
        field('control', $.control_path),
        '=',
        field('value', choice($.position, $.number)),
      ),

    verb_step: $ =>
      seq(
        field('verb', choice('start', 'stop', 'open', 'close')),
        field('control', $.control_path),
      ),

    notify_step: $ => seq('notify', field('message', $.string)),

    call_step: $ => seq('call', field('target', $.identifier)),

    wait_step: $ =>
      seq('wait', field('condition', $._expression), optional(field('timeout', $.timeout_clause))),

    if_step: $ =>
      seq(
        'if',
        field('condition', $._expression),
        field('consequence', $.block),
        optional(seq('else', field('alternative', choice($.if_step, $.block)))),
      ),

    complete_step: $ =>
      seq(
        'complete',
        optional(
          choice(
            seq(
              'when',
              field('condition', $._expression),
              optional(field('timeout', $.timeout_clause)),
            ),
            field('timeout', $.timeout_clause),
          ),
        ),
      ),

    fail_step: $ => seq('fail', field('message', optional($.string))),

    timeout_clause: $ =>
      seq('timeout', field('duration', $.duration), optional(seq('else', 'fail'))),

    block: $ => seq('{', repeat($.step), '}'),

    // ------------------------------------------------------------- expressions

    _expression: $ =>
      choice(
        $.binary_expression,
        $.unary_expression,
        $.parenthesized_expression,
        $.boolean,
        $.number,
        $.path,
      ),

    binary_expression: $ =>
      choice(
        prec.left(1, seq($._expression, field('operator', '||'), $._expression)),
        prec.left(2, seq($._expression, field('operator', '&&'), $._expression)),
        prec.left(
          3,
          seq(
            $._expression,
            field('operator', choice('<', '<=', '>', '>=', '==', '!=')),
            $._expression,
          ),
        ),
      ),

    unary_expression: $ =>
      prec(
        4,
        choice(
          seq(field('operator', '!'), $._expression),
          seq(field('operator', '-'), $.number),
        ),
      ),

    parenthesized_expression: $ => seq('(', $._expression, ')'),

    // ------------------------------------------------------------------ tokens

    // A registry lookup key, written exactly as the host registered it:
    // `hydraulic.2.pressure`. A read of aircraft state.
    path: $ => seq($.identifier, repeat(seq('.', choice($.identifier, $.number)))),

    // The same shape, but naming a control rather than a state symbol. It is a
    // separate node so that a highlight query can colour the thing a procedure
    // *moves* differently from the thing it *reads*, without the two patterns
    // overlapping and leaving the winner up to the editor.
    control_path: $ => seq($.identifier, repeat(seq('.', choice($.identifier, $.number)))),

    // The right-hand side of `set` — a position name such as `ON` or
    // `TANK_3_TO_1`. Matched case-insensitively by the compiler.
    position: $ => $.identifier,

    boolean: $ => choice('true', 'false'),

    identifier: $ => /[A-Za-z_][A-Za-z0-9_]*/,

    number: $ => /[0-9]+(\.[0-9]+)?/,

    duration: $ => /[0-9]+(\.[0-9]+)?(ms|s|m)/,

    string: $ => seq('"', repeat(choice($.escape_sequence, /[^"\\\n]+/)), '"'),

    escape_sequence: $ => /\\["\\nt]/,

    line_comment: $ => token(seq('//', /[^\n]*/)),

    block_comment: $ => token(seq('/*', /[^*]*\*+([^/*][^*]*\*+)*/, '/')),
  },
});
