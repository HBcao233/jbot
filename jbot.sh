#!/bin/bash
root=$(dirname $(realpath $0))

color=("\e[31m" "\e[32m")
status_text=("Died" "Runing")

p=$(ps aux | grep "/jbot" | grep -v grep | grep -v ps | grep -v sh | awk '{print $2}')
pid=$p
status=0
start_time=""
if [[ ! -z "$p" ]]; then
    status=1
    start_time=$(ps -p $p -o lstart | sed -n '2p')
fi

_status(){
    s=${status}
    printf ${color[$s]}
    printf "●"
    printf "\e[0m"
    printf " jbot   "
    printf ${color[$s]}
    printf ${status_text[$s]}
    printf "\e[0m\n"

    printf "%10s %s\n" PID "${pid}"
    printf "%10s %s\n" Since "${start_time}"
}

_stop(){
    if [ ${status} -eq 0 ]; then
        echo "jbot 未启动"
    else
        p=${pid}
        kill -SIGINT "$p"
        echo -n "杀死进程 $p"

        local begin=$(date +%s)
        local end
        while kill -0 "$pid" > /dev/null 2>&1
        do
            echo -n "."
            sleep 0.1;

            end=$(date +%s)
            if [ $((end-begin)) -gt 2  ]; then
                echo -e "\nTimeout"
                break;
            fi
        done
        echo

        pid=0
        status=0
        start_time=""
    fi
}

_start(){
    if [ ${status} -gt 0 ]; then
        echo "jbot 已经正在运行啦"
    else
        cd $root && nohup $root/target/release/jbot > $root/bot.log 2>&1 &
        echo "启动 jbot 中..."
    fi
}

action="$1"

case $action in
    start)
        _start
        ;;
    stop)
        _stop
        ;;
    restart)
        _stop
        _start
        ;;
    status)
        _status
        ;;
    log)
        num=100
        if [ -n "$2" ]; then
            num=$2
        fi
        printf '%b' "$( \
            tail -n $num $root/bot.log \
            | sed -e 's/\\/\\\\/g' \
                  -e "s/\(DEBUG\)/\\\033[35m\1\\\033[00m/g" \
                  -e "s/\(INFO\)/\\\033[36m\1\\\033[00m/g" \
                  -e "s/\(WARN\)/\\\033[33m\1\\\033[00m/g" \
                  -e "s/\(ERROR\)/\\\033[31m\1\\\033[00m/g" \
        )"
        ;;
    ps)
        ps aux | grep "/jbot" | grep -v grep | grep -v ps | grep -v sh
        ;;
    *)
        echo "Usage: $0 <status|start|stop|restart|log|ps>"
        exit 1
esac
exit 0
