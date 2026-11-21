# The GoldLink relay in a container, from the binary beside this file (README.md, "Docker").
#   docker build -t goldlink-relayserver .
#   docker run -d --name goldlink-relayserver --restart unless-stopped -p 7777:7777/udp -v goldlink-relayserver:/data goldlink-relayserver
FROM alpine:3.20
COPY GoldLink-RelayServer /usr/local/bin/GoldLink-RelayServer
RUN adduser -D -H -u 10001 relay && mkdir /data && chown relay /data
USER relay
VOLUME /data
EXPOSE 7777/udp
ENTRYPOINT ["/usr/local/bin/GoldLink-RelayServer", "--bind", "0.0.0.0:7777", "--save-dir", "/data"]
