import { credentials } from "@grpc/grpc-js";
import { MTransactionClient } from './proto/mtransaction_grpc_pb';
import { EchoRequest, EchoResponse } from "./proto/mtransaction_pb";

function main() {
  var target = 'http://192.168.200.176:50051/';
  var client = new MTransactionClient(target, credentials.createInsecure());
  client.echo(new EchoRequest(), function(err: unknown, response: EchoResponse) {
    if (err) throw err;
    console.log('Greeting:', response.getMessage());
    // client.close();
  });
}

main();