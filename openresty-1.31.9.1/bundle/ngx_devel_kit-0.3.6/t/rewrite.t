use Test::Nginx::Socket;

repeat_each(1);

plan tests => repeat_each() * blocks() * 3;

run_tests();

__DATA__

=== TEST 1: literal arguments
--- config
    location /t {
        set_concat2 $joined left right;
        return 200 "$joined\n";
    }
--- request
GET /t
--- response_body
leftright
--- no_error_log
[error]


=== TEST 2: two variable arguments
--- config
    location /t {
        set_concat2 $joined $arg_a $arg_b;
        return 200 "$joined\n";
    }
--- request
GET /t?a=left&b=right
--- response_body
leftright
--- no_error_log
[error]


=== TEST 3: literal followed by a variable
--- config
    location /t {
        set_concat2 $joined prefix- $arg_value;
        return 200 "$joined\n";
    }
--- request
GET /t?value=right
--- response_body
prefix-right
--- no_error_log
[error]


=== TEST 4: variable followed by a literal
--- config
    location /t {
        set_concat2 $joined $arg_value -suffix;
        return 200 "$joined\n";
    }
--- request
GET /t?value=left
--- response_body
left-suffix
--- no_error_log
[error]


=== TEST 5: compound expression
--- config
    location /t {
        set_concat2 $joined "$arg_a:$arg_b" :tail;
        return 200 "$joined\n";
    }
--- request
GET /t?a=left&b=right
--- response_body
left:right:tail
--- no_error_log
[error]


=== TEST 6: chained assignments
--- config
    location /t {
        set_concat2 $first $arg_a $arg_b;
        set_concat2 $joined $first $arg_c;
        return 200 "$joined\n";
    }
--- request
GET /t?a=left&b=middle&c=right
--- response_body
leftmiddleright
--- no_error_log
[error]


=== TEST 7: empty variable arguments
--- config
    location /t {
        set_concat2 $joined $arg_a ":$arg_b";
        return 200 "$joined\n";
    }
--- request
GET /t
--- response_body
:
--- no_error_log
[error]


=== TEST 8: regex capture arguments
--- config
    location /t/ {
        if ($uri ~ ^/t/([^/]+)/([^/]+)$) {
            set_concat2 $joined $1 $2;
            return 200 "$joined\n";
        }
    }
--- request
GET /t/left/right
--- response_body
leftright
--- no_error_log
[error]
