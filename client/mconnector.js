const fs = require('fs');
const web3 = require('@solana/web3.js')
const path = require("path");
const protoLoader = require('@grpc/proto-loader');
const winston = require('winston');

const PROTO_PATH = path.join(__dirname, '..', 'proto', 'mtransaction.proto');
const packageDefinition = protoLoader.loadSync(PROTO_PATH, {keepCase: true});

const grpc = require('@grpc/grpc-js');
const validatorProto = grpc.loadPackageDefinition(packageDefinition).validator;

const getEnvironmentVariable = (key) => {
  const val = process.env[key]
  if (val === undefined) {
    throw new Error(`Environment variable ${key} must be defined!`)
  }
  return val
}

const Logger = winston.createLogger({
    level: 'info',
    format: winston.format.combine(
        winston.format.timestamp(),
        winston.format.colorize(),
        winston.format.simple()
    ),
    defaultMeta: { service: 'mtx-client-js' },
    transports: [new winston.transports.Console()],
});

process.on('uncaughtException', (err) => {
    console.log('Caught exception: ' + err + err.stack)
})

class Metrics {
    tx_received = 0
    tx_succeeded = 0
    tx_failed = 0
}

function connect(millisecondsToWait) {
    const metrics = new Metrics()
    const logger = Logger.child({ connection: Math.random().toString(36).slice(2) })
    const cluster = new web3.Connection(getEnvironmentVariable('SOLANA_CLUSTER_URL'))
    const ssl_creds = grpc.credentials.createSsl(
        fs.readFileSync(getEnvironmentVariable('TLS_GRPC_SERVER_CERT')),
        fs.readFileSync(getEnvironmentVariable('TLS_GRPC_CLIENT_KEY')),
        fs.readFileSync(getEnvironmentVariable('TLS_GRPC_CLIENT_CERT')),
    );
    const mtransactionClient = new validatorProto.MTransaction(getEnvironmentVariable('GRPC_SERVER_ADDR'), ssl_creds);
    const call = mtransactionClient.TxStream();

    const sendPong = (id) => {
      logger.info('Sending pong', { id })
      call.write({ pong: { id } })
    }
    const sendMetrics = () => {
      logger.info('Sending pong', { metrics })
      call.write({ metrics })
    }

    const processTx = ({ data }) => {
        metrics.tx_received++
        logger.info('Received tx', { data })
        try {
            // cluster.sendRawTransaction(Buffer.from(data, 'base64'), { preflightCommitment: 'processed' }).then((v) => console.log(r, 'tx', v))
            metrics.tx_succeeded++
        } catch (err) {
            logger.error('Failed to process tx', { err })
            metrics.tx_failed++
        }
    }
    const processPing = ({ id }) => {
        logger.info('Received ping', { id })
        sendPong(id)
    }

    const scheduleMetrics = () => setTimeout(() => {
        try {
            if (!call._readableState.ended) {
                sendMetrics()
                scheduleMetrics()
            }
        } catch (err) {
            logger.error('Failed to send metrics', { err })
        }
    }, 15000)
    scheduleMetrics()

    function restart(millisecondsToWait) {
        logger.warn('Will restart the stream', { millisecondsToWait })
        setTimeout(() => connect(millisecondsToWait), millisecondsToWait);
    }

    call.on('data', ({ tx, ping }) => {
        if (tx) {
            processTx(tx)
        }
        if (ping) {
            processPing(ping)
        }
        millisecondsToWait = 500;
    });
    call.on('end', () => {
        millisecondsToWait += millisecondsToWait;
        mtransactionClient.close();
        logger.error('Connection has been closed.')
        restart(millisecondsToWait);
    });
    call.on('error', (err) => {
        logger.error('Connection error.', { err })
    });
}

function main() {
    connect(500);
}

main();
