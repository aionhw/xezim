

module tb;
    // Declare a dynamic array of mailboxes
    mailbox #(int) mbx_array[][16];

    initial begin
        // Allocate memory for the dynamic array and initialize each mailbox
        mbx_array = new[5];
        foreach (mbx_array[i,j]) begin
            mbx_array[i][j] = new();
        end

        // Put data into each mailbox
        foreach (mbx_array[i,x]) begin
                        int data ;
                        data = i + x ;
                mbx_array[i][x].put(data);
        end

        // Retrieve data from each mailbox
        foreach (mbx_array[i,x]) begin
                int data;
            mbx_array[i][x].get(data);
            $display("Data from mbx_array[%0d][%0d] = %0d", i, x, data);
        end
    end
endmodule

